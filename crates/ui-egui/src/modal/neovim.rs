//! External editor transport through a clean Neovim client and its documented RPC API.
//! No editor code is linked or bundled. See https://neovim.io/doc/user/remote.html and api.html.
//! All process and file work runs on the worker, never on the drawing thread.

use std::fs::{self, File, OpenOptions};
use std::io::Read;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use serde_json::Value;

const LIMIT: u64 = 32 * 1024 * 1024;
static NEXT: AtomicU64 = AtomicU64::new(0);

pub(super) struct Worker {
    pub requests: mpsc::SyncSender<Value>,
    pub replies: mpsc::Receiver<Result<Value, String>>,
    pub geometry: mpsc::SyncSender<Result<Value, String>>,
    cancelled: Arc<AtomicBool>,
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }
}

impl Worker {
    #[cfg(test)]
    pub(super) fn fake() -> (Self, mpsc::Receiver<Value>, mpsc::Sender<Result<Value, String>>) {
        let (requests, incoming) = mpsc::sync_channel(1);
        let (outgoing, replies) = mpsc::channel();
        let (geometry, _) = mpsc::sync_channel(1);
        (Self { requests, replies, geometry, cancelled: Arc::new(AtomicBool::new(false)) }, incoming, outgoing)
    }

    pub fn start(config: String, ctx: egui::Context) -> Result<Self, String> {
        let (requests, incoming) = mpsc::sync_channel(1);
        let (outgoing, replies) = mpsc::channel();
        let (geometry, answers) = mpsc::sync_channel(1);
        let cancelled = Arc::new(AtomicBool::new(false));
        let stop = cancelled.clone();
        std::thread::Builder::new()
            .name("wordcraft-modal".into())
            .spawn(move || {
                let mut editor = match Editor::start(&config, stop.clone()) {
                    Ok(editor) => editor,
                    Err(error) => {
                        let _ = outgoing.send(Err(error));
                        ctx.request_repaint();
                        return;
                    }
                };
                while !stop.load(Ordering::Relaxed) {
                    let request = match incoming.recv_timeout(Duration::from_millis(100)) {
                        Ok(request) => request,
                        Err(mpsc::RecvTimeoutError::Timeout) => continue,
                        Err(_) => break,
                    };
                    let result = editor.request(&request, &mut |query| {
                        outgoing.send(Ok(serde_json::json!({"layoutQuery":query}))).map_err(|e| e.to_string())?;
                        ctx.request_repaint();
                        let deadline = Instant::now() + Duration::from_secs(4);
                        loop {
                            match answers.recv_timeout(Duration::from_millis(20)) {
                                Ok(answer) => return answer,
                                Err(mpsc::RecvTimeoutError::Timeout) if !stop.load(Ordering::Relaxed) && Instant::now() < deadline => {}
                                Err(_) => return Err("document layout did not answer in time".into()),
                            }
                        }
                    });
                    let failed = result.is_err();
                    if outgoing.send(result).is_err() {
                        break;
                    }
                    ctx.request_repaint();
                    if failed {
                        break;
                    }
                }
            })
            .map_err(|error| format!("could not start the editor worker: {error}"))?;
        Ok(Self { requests, replies, geometry, cancelled })
    }
}

struct Editor {
    directory: PathBuf,
    socket: PathBuf,
    child: Option<Child>,
    cancelled: Arc<AtomicBool>,
    input: String,
}

impl Drop for Editor {
    fn drop(&mut self) {
        if let Some(child) = &mut self.child {
            let _ = child.kill();
            let _ = child.wait();
        }
        let _ = fs::remove_dir_all(&self.directory);
    }
}

impl Editor {
    fn start(config: &str, cancelled: Arc<AtomicBool>) -> Result<Self, String> {
        let directory = std::env::temp_dir().join(format!("wordcraft-modal-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        let mut builder = fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder.create(&directory).map_err(|e| e.to_string())?;
        let socket = directory.join("editor.sock");
        let mut editor = Self { directory, socket, child: None, cancelled, input: String::new() };
        let mut command = Command::new("nvim");
        command.args(["--headless", "-i", "NONE", "--listen"]).arg(&editor.socket);
        if !config.is_empty() {
            let directory = PathBuf::from(config);
            let init = [directory.join("init.lua"), directory.join("init.vim")]
                .into_iter()
                .find(|p| p.is_file())
                .ok_or_else(|| "the editor configuration folder has no init.lua or init.vim".to_string())?;
            if let (Some(parent), Some(name)) = (directory.parent(), directory.file_name()) {
                command.env("XDG_CONFIG_HOME", parent).env("NVIM_APPNAME", name);
            }
            command.arg("-u").arg(init);
        }
        editor.child = Some(
            command
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .map_err(|e| format!("could not start the external editor: {e}"))?,
        );
        let deadline = Instant::now() + Duration::from_secs(15);
        while !editor.socket.exists() {
            if editor.cancelled.load(Ordering::Relaxed) || Instant::now() >= deadline {
                return Err("the external editor did not start in time".into());
            }
            if editor.child.as_mut().and_then(|p| p.try_wait().ok().flatten()).is_some() {
                return Err("the external editor exited during startup".into());
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        fs::write(editor.directory.join("init.lua"), INIT).map_err(|e| e.to_string())?;
        fs::write(editor.directory.join("client.lua"), CLIENT).map_err(|e| e.to_string())?;
        editor.request(&serde_json::json!({"initialize":true}), &mut |_| Err("layout is not ready".into()))?;
        Ok(editor)
    }

    fn request(&mut self, request: &Value, layout: &mut dyn FnMut(Value) -> Result<Value, String>) -> Result<Value, String> {
        let query_path = self.directory.join("layout-request.json");
        let answer_path = self.directory.join("layout-reply.json");
        let _ = fs::remove_file(&query_path);
        let _ = fs::remove_file(&answer_path);
        let mut request = request.clone();
        self.input.push_str(request["keys"].as_str().unwrap_or(""));
        request["keys"] = Value::String(self.input.clone());
        let path = self.directory.join("request.json");
        fs::write(&path, serde_json::to_vec(&request).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
        let reply = self.directory.join("reply.json");
        let output = OpenOptions::new().create(true).truncate(true).write(true).open(&reply).map_err(|e| e.to_string())?;
        let mut client = Command::new("nvim")
            .args(["--clean", "--headless", "-l"])
            .arg(self.directory.join("client.lua"))
            .arg(&self.socket)
            .arg(&path)
            .arg(self.directory.join("init.lua"))
            .stdin(Stdio::null())
            .stdout(output)
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| e.to_string())?;
        let mut deadline = Instant::now() + Duration::from_secs(5);
        let mut queries = 0;
        let status = loop {
            if query_path.exists() {
                queries += 1;
                if queries > 128 {
                    let _ = client.kill();
                    let _ = client.wait();
                    return Err("too many displayed-line operations in one editor request".into());
                }
                let result = (|| {
                    let mut bytes = Vec::new();
                    File::open(&query_path).map_err(|e| e.to_string())?.take(LIMIT + 1).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
                    fs::remove_file(&query_path).map_err(|e| e.to_string())?;
                    if bytes.len() as u64 > LIMIT {
                        return Err("layout request exceeded the size limit".into());
                    }
                    let query: Value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
                    let id = query["id"].clone();
                    let mut answer = match layout(query) {
                        Ok(answer) => answer,
                        Err(error) => serde_json::json!({"error":error}),
                    };
                    answer["id"] = id;
                    let temporary = self.directory.join("layout-reply.new");
                    fs::write(&temporary, serde_json::to_vec(&answer).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
                    fs::rename(temporary, &answer_path).map_err(|e| e.to_string())?;
                    Ok::<_, String>(())
                })();
                if let Err(error) = result {
                    let _ = client.kill();
                    let _ = client.wait();
                    return Err(error);
                }
                deadline = Instant::now() + Duration::from_secs(5);
            }
            match client.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) => {}
                Err(error) => {
                    let _ = client.kill();
                    let _ = client.wait();
                    return Err(error.to_string());
                }
            }
            if self.cancelled.load(Ordering::Relaxed) || Instant::now() >= deadline {
                let _ = client.kill();
                let _ = client.wait();
                return Err("the external editor stopped responding; the document was not overwritten".into());
            }
            std::thread::sleep(Duration::from_millis(5));
        };
        if !status.success() {
            return Err("the external editor refused the request".into());
        }
        let mut bytes = Vec::new();
        File::open(reply).map_err(|e| e.to_string())?.take(LIMIT + 1).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
        if bytes.len() as u64 > LIMIT {
            return Err("the external editor reply exceeded the size limit".into());
        }
        let mut reply: Value = serde_json::from_slice(&bytes).map_err(|e| format!("invalid external editor reply: {e}"))?;
        let accepted = reply["accepted"].as_u64().and_then(|v| usize::try_from(v).ok()).ok_or("the editor returned no input acknowledgement")?;
        if !self.input.is_char_boundary(accepted) {
            return Err("the editor returned an invalid input acknowledgement".into());
        }
        self.input.drain(..accepted);
        reply["inputPending"] = Value::Bool(reply["busy"].as_bool() == Some(true) || !self.input.is_empty());
        Ok(reply)
    }
}

// The client only uses fast RPC while the editor waits for an operator, count or prompt.
// Deferred calls there deadlock: the GUI must be able to send the next key first.
const CLIENT: &str = r#"
local channel=vim.fn.sockconnect('pipe',arg[1],{rpc=true})
if channel==0 then error('could not connect to the editor') end
local r=vim.json.decode(table.concat(vim.fn.readfile(arg[2]),'\n'))
local function rpc(method,...) return vim.rpcrequest(channel,method,...) end
if r.initialize then
  local code=table.concat(vim.fn.readfile(arg[3]),'\n')
  rpc('nvim_exec_lua',code,{vim.fn.fnamemodify(arg[3],':h')})
  io.write(vim.json.encode({accepted=0})); return
end
local function mode() return rpc('nvim_get_mode') end
if r.sync and not r.keepInsert then
  rpc('nvim_input','<Esc>')
  if not vim.wait(500,function() return not mode().blocking end,10) then error('the editor cannot resynchronise yet') end
end
local prepared=false
if not mode().blocking then
  if not rpc('nvim_exec_lua','return WordCraftModal.request(...)',{r}) then
    local result=rpc('nvim_exec_lua','return WordCraftModal.snapshot()',{})
    result.accepted=0
    io.write(vim.json.encode(result)); return
  end
  prepared=true
end
local accepted=0
if #r.keys>0 then accepted=rpc('nvim_input',r.keys) end
vim.wait(25)
local current=mode()
local result
if current.blocking then
  result={busy=true,mode=current.mode}
else
  if not prepared then
    r.sync=false; r.place=false; r.keys=''
    rpc('nvim_exec_lua','return WordCraftModal.request(...)',{r})
  end
  repeat
    local state=mode()
    if state.blocking then result={busy=true,mode=state.mode}; break end
    result=rpc('nvim_exec_lua','return WordCraftModal.snapshot()',{})
    if result.layoutPending then vim.wait(10) end
  until not result.layoutPending
end
result.accepted=accepted
io.write(vim.json.encode(result))
"#;

// This is integration code against the public API, not copied editor implementation code.
const INIT: &str = r#"
local directory=...
(function()
  if package.loaded.lazy then
    vim.api.nvim_exec_autocmds('User', {pattern='VeryLazy', modeline=false})
  end
  local m = {base='', save=false}
  WordCraftModal = m
  m.buf = vim.api.nvim_create_buf(true, false)
  vim.api.nvim_set_current_buf(m.buf)
  vim.api.nvim_buf_set_name(m.buf, 'wordcraft://document')
  vim.bo[m.buf].buftype = 'acwrite'
  vim.bo[m.buf].swapfile = false
  vim.bo[m.buf].bufhidden = 'hide'
  vim.bo[m.buf].filetype = 'wordcraft'
  vim.api.nvim_create_autocmd('BufWriteCmd', {buffer=m.buf, callback=function(event)
    m.save=true
    if event.match~=vim.api.nvim_buf_get_name(m.buf) then m.savePath=event.match end
  end})
  vim.api.nvim_create_autocmd('QuitPre', {buffer=m.buf, callback=function()
    m.problem='Close documents in WordCraft so its save prompts can protect your work.'
    error(m.problem)
  end})
  local function offset(pos)
    local lines = vim.api.nvim_buf_get_lines(m.buf, 0, -1, false)
    local at = 0
    for i=1, math.min(pos[1]-1, #lines) do at=at+#lines[i]+1 end
    return at+pos[2]
  end
  local function position(at)
    local lines = vim.api.nvim_buf_get_lines(m.buf, 0, -1, false)
    for i,line in ipairs(lines) do
      if at<=#line then return {i,at} end
      at=at-#line-1
    end
    return {#lines,#lines[#lines]}
  end
  local function previous(text,at)
    if at<=0 then return 0 end
    at=at-1
    while at>0 and text:byte(at+1)>=128 and text:byte(at+1)<=191 do at=at-1 end
    return at
  end
  local function go(at) vim.api.nvim_win_set_cursor(0,position(at)) end
  local function select(a,b)
    vim.cmd('normal! '..string.char(27))
    go(a); vim.cmd('normal! v'); go(b)
  end
  function m.geometry(action,count,at,goal)
    m.query=(m.query or 0)+1
    local id=m.query
    local request={id=id,action=action,count=count or 1,at=at or offset(vim.api.nvim_win_get_cursor(0)),goal=goal,snapshot=m.snapshot()}
    m.geometryActive=true
    local temporary=directory..'/layout-request.new'
    vim.fn.writefile({vim.json.encode(request)},temporary)
    assert(vim.uv.fs_rename(temporary,directory..'/layout-request.json'))
    local answer
    local ready=vim.wait(4000,function()
      local ok,lines=pcall(vim.fn.readfile,directory..'/layout-reply.json')
      if not ok then return false end
      local decoded,value=pcall(vim.json.decode,table.concat(lines,'\n'))
      if decoded and value.id==id then answer=value; return true end
      return false
    end,10,true)
    m.geometryActive=false
    if not ready or answer.error then
      m.problem=answer and answer.error or 'WordCraft did not provide the displayed line in time.'
      error(m.problem)
    end
    m.base=answer.text
    return answer
  end
  local function leave_row_visual(mode)
    if mode~='v' and mode~='V' and mode~='\022' and m.rowVisual~=nil then
      m.rowVisual=nil; m.goal=nil; m.lastVertical=nil
    end
  end
  local function vertical(direction)
    leave_row_visual(vim.api.nvim_get_mode().mode)
    local at=offset(vim.api.nvim_win_get_cursor(0))
    local goal=m.lastVertical==at and m.goal or nil
    local result=m.geometry(direction,vim.v.count1,at,goal)
    if m.rowVisual then
      local origin=m.geometry('row',1,m.rowVisual)
      local target=m.geometry('row',1,result.focus)
      if target.start>=origin.start then select(origin.start,previous(m.base,target.finish))
      else select(previous(m.base,origin.finish),target.start) end
    else go(result.focus) end
    m.goal=result.goal; m.lastVertical=offset(vim.api.nvim_win_get_cursor(0))
  end
  local function bound(which)
    m.rowVisual=nil; m.goal=nil
    local result=m.geometry('row',vim.v.count1)
    local at=result.start
    if which=='end' then at=previous(m.base,result.finish)
    elseif which=='first' then
      local prefix=m.base:sub(result.start+1,result.finish):match('^%s*') or ''
      at=result.start+#prefix
    end
    go(at)
  end
  local function visual_row()
    local result=m.geometry('row',vim.v.count1)
    m.rowVisual=result.start; m.goal=nil
    select(result.start,previous(m.base,result.finish))
  end
  local function replace_range(a,b,text)
    local first,last=position(a),position(b)
    vim.api.nvim_buf_set_text(m.buf,first[1]-1,first[2],last[1]-1,last[2],vim.split(text,'\n',{plain=true}))
    go(a)
  end
  local function yank(a,b,register)
    local text=m.base:sub(a+1,b)
    register=register or vim.v.register
    if register=='_' then return end
    vim.fn.setreg(register,text,'vu')
    m.rowRegister={register=register,text=vim.fn.getreg(register)}
  end
  local function row_range(kind,count,content)
    local at=offset(vim.api.nvim_win_get_cursor(0))
    local origin=at
    if kind=='up' then origin=m.geometry('up',count,at).focus end
    local result=m.geometry('row',(kind=='up' or kind=='down') and count+1 or count,origin)
    local first,last=result.start,content and result.finish or result.eraseEnd
    if kind=='tail' then first=at; last=result.finish
    elseif kind=='start' then last=at
    elseif kind=='first' then
      first=result.start+#(m.base:sub(result.start+1,result.finish):match('^%s*') or '')
      first,last=math.min(first,at),math.max(first,at)
    end
    return first,last
  end
  function m.delete_row()
    local operation=m.rowOperation
    local first,last=row_range(operation.kind,operation.count,false)
    yank(first,last,operation.register)
    replace_range(first,last,'')
    m.rowVisual=nil; m.goal=nil
  end
  local function prepare_delete(kind,count,register)
    m.rowOperation={kind=kind,count=count or vim.v.count1,register=register or vim.v.register}
    vim.go.operatorfunc='v:lua.WordCraftModal.delete_row'
    return 'g@l'
  end
  local function yank_rows()
    local first,last=row_range('rows',vim.v.count1,false)
    yank(first,last)
    m.rowVisual=nil; go(first)
  end
  local function change_rows(kind,count,register)
    count=count or vim.v.count1
    register=register or vim.v.register
    local first,last=row_range(kind,count,true)
    m.pendingRowChange={count=count,kind=kind,register=register}
    if first==last then
      go(first)
      local point=position(first)
      local line=vim.api.nvim_buf_get_lines(m.buf,point[1]-1,point[1],false)[1]
      vim.cmd(point[2]>=#line and 'startinsert!' or 'startinsert')
    else
      select(first,previous(m.base,last))
      -- Queue the native change before pending typing; its Insert session stays one undo step.
      vim.api.nvim_feedkeys('"'..register..'c','in',false)
    end
    m.rowVisual=nil; m.goal=nil
  end
  local function pending_motion(kind)
    local operator,count,register=vim.v.operator,vim.v.count1,vim.v.register
    vim.cmd('normal! '..string.char(27))
    if operator=='d' then
      local keys=prepare_delete(kind,count,register)
      vim.api.nvim_feedkeys(keys,'in',false)
    elseif operator=='c' then change_rows(kind,count,register)
    else
      local first,last=row_range(kind,count,false)
      if operator=='y' then yank(first,last,register); go(first)
      elseif first<last then
        select(first,previous(m.base,last))
        vim.api.nvim_feedkeys('"'..register..operator,'in',false)
      end
    end
  end
  local function paste_row(before)
    local register,count=vim.v.register,vim.v.count1
    local operation=before and 'P' or 'p'
    if m.rowRegister and vim.fn.getreg(register)==m.rowRegister.text then
      local result=m.geometry('row',1)
      local at=before and result.start or result.eraseEnd
      local point=position(at)
      local line=vim.api.nvim_buf_get_lines(m.buf,point[1]-1,point[1],false)[1]
      if point[2]==#line and at==#m.base then
        go(previous(m.base,at)); operation='p'
      else go(at); operation='P' end
    end
    vim.api.nvim_feedkeys('"'..register..tostring(count)..operation,'in',false)
  end
  vim.api.nvim_create_autocmd('InsertLeave',{buffer=m.buf,callback=function()
    if m.pendingRowChange then
      m.lastRowChange=m.pendingRowChange; m.pendingRowChange=nil
      m.lastRowChange.text=vim.fn.getreg('.')
      m.lastRowChange.sequence=vim.fn.undotree().seq_last
    end
  end})
  local function repeat_change()
    local last=m.lastRowChange
    if not last or vim.fn.undotree().seq_last~=last.sequence then
      vim.cmd('normal! '..(vim.v.count>0 and tostring(vim.v.count) or '')..'.'); return
    end
    local first,finish=row_range(last.kind,vim.v.count>0 and vim.v.count or last.count,true)
    yank(first,finish,last.register)
    replace_range(first,finish,last.text)
    go(previous(table.concat(vim.api.nvim_buf_get_lines(m.buf,0,-1,false),'\n'),first+#last.text))
    last.sequence=vim.fn.undotree().seq_last
  end
  local options={buffer=m.buf,silent=true}
  vim.keymap.set({'n','x'},'j',function() vertical('down') end,options)
  vim.keymap.set({'n','x'},'k',function() vertical('up') end,options)
  vim.keymap.set({'n','x'},'gj',function() vertical('down') end,options)
  vim.keymap.set({'n','x'},'gk',function() vertical('up') end,options)
  for _,key in ipairs({'0','g0'}) do vim.keymap.set({'n','x'},key,function() bound('start') end,options) end
  for _,key in ipairs({'$','g$'}) do vim.keymap.set({'n','x'},key,function() bound('end') end,options) end
  vim.keymap.set({'n','x'},'^',function() bound('first') end,options)
  vim.keymap.set({'n','x'},'gg',function()
    local result=m.geometry('first',vim.v.count1); go(result.focus); m.rowVisual=nil
  end,options)
  vim.keymap.set({'n','x'},'G',function()
    local result=m.geometry(vim.v.count>0 and 'first' or 'last',vim.v.count1); go(result.focus); m.rowVisual=nil
  end,options)
  for key,kind in pairs({j='down',k='up',gj='down',gk='up',['0']='start',g0='start',['^']='first',['$']='tail',['g$']='tail'}) do
    vim.keymap.set('o',key,function() pending_motion(kind) end,options)
  end
  vim.keymap.set('n','V',visual_row,options)
  vim.keymap.set('n','dd',function() return prepare_delete('rows') end,{buffer=m.buf,silent=true,expr=true})
  vim.keymap.set('n','yy',yank_rows,options)
  vim.keymap.set('n','Y',yank_rows,options)
  vim.keymap.set('n','cc',function() change_rows('rows') end,options)
  vim.keymap.set('n','S',function() change_rows('rows') end,options)
  vim.keymap.set('n','D',function() return prepare_delete('tail') end,{buffer=m.buf,silent=true,expr=true})
  vim.keymap.set('n','C',function() change_rows('tail') end,options)
  vim.keymap.set('n','.',repeat_change,options)
  vim.keymap.set('n','p',function() paste_row(false) end,options)
  vim.keymap.set('n','P',function() paste_row(true) end,options)

  function m.request(r)
    if vim.api.nvim_get_current_buf()~=m.buf then return false end
    if r.sync then
      m.rowVisual=nil; m.goal=nil
      if not r.keepInsert then vim.cmd('stopinsert') end
      local undo = vim.bo[m.buf].undolevels
      vim.bo[m.buf].undolevels = -1
      vim.api.nvim_buf_set_lines(m.buf,0,-1,false,vim.split(r.text,'\n',{plain=true}))
      vim.bo[m.buf].undolevels = undo
      local pos=position(r.focus)
      vim.api.nvim_win_set_cursor(0,pos)
    elseif r.place then
      vim.api.nvim_win_set_cursor(0,position(r.focus))
    end
    m.base=r.text
    if r.saved then vim.bo[m.buf].modified=false end
    return true
  end
  local function tokens(text)
    local chars={}
    for c in text:gmatch('[%z\1-\127\194-\244][\128-\191]*') do
      chars[#chars+1]=c:gsub('.',function(b) return string.format('%02x',string.byte(b)) end)
    end
    return table.concat(chars,'\n')..'\n'
  end
  local function boundary(text, index)
    if index<=0 then return 0 end
    local count=0
    local at=1
    for c in text:gmatch('[%z\1-\127\194-\244][\128-\191]*') do
      if count==index then return at-1 end
      count=count+1
      at=at+#c
    end
    return #text
  end
  function m.snapshot()
    local other=vim.api.nvim_get_current_buf()~=m.buf
    if other then m.problem='Open and switch documents in WordCraft; editor plug-in buffers are not document pages.' end
    local text=table.concat(vim.api.nvim_buf_get_lines(m.buf,0,-1,false),'\n')
    if #text>2097152 then error('the text buffer is too large') end
    local edits={}
    if m.base~=text then
      for _,h in ipairs(vim.diff(tokens(m.base),tokens(text),{result_type='indices'})) do
        local old=h[2]==0 and h[1] or h[1]-1
        local new=h[4]==0 and h[3] or h[3]-1
        edits[#edits+1]={start=boundary(m.base,old),['end']=boundary(m.base,old+h[2]),
          text=text:sub(boundary(text,new)+1,boundary(text,new+h[4]))}
      end
    end
    local mode=other and 'n' or vim.api.nvim_get_mode().mode
    leave_row_visual(mode)
    local cursor=other and (m.cursor or {1,0}) or vim.api.nvim_win_get_cursor(0)
    if not other then m.cursor=cursor end
    local focus=math.min(offset(cursor),#text)
    while focus>0 and text:byte(focus+1) and text:byte(focus+1)>=128 and text:byte(focus+1)<=191 do focus=focus-1 end
    local caret=focus
    local anchor=focus
    if mode=='v' or mode=='V' or mode=='\022' then
      local p=vim.fn.getpos('v')
      anchor=offset({p[2],math.max(0,p[3]-1)})
      if mode=='V' then
        local a,b=position(anchor),position(focus)
        local first,last=math.min(a[1],b[1]),math.max(a[1],b[1])
        local lines=vim.api.nvim_buf_get_lines(m.buf,0,-1,false)
        anchor=offset({first,0}); focus=offset({last,#lines[last]})
      else
        local at=math.max(anchor,focus)
        local ch=text:sub(at+1):match('^[%z\1-\127\194-\244][\128-\191]*')
        if focus>=anchor then focus=focus+(ch and #ch or 0) else anchor=anchor+(ch and #ch or 0) end
      end
    end
    local save,path,problem=m.save,m.savePath,m.problem; m.save=false; m.savePath=nil; m.problem=nil
    local sequence=vim.api.nvim_buf_call(m.buf,function() return vim.fn.undotree().seq_cur end)
    return {text=text,edits=edits,focus=focus,anchor=anchor,caret=caret,mode=mode,sequence=sequence,layoutPending=m.geometryActive or vim.fn.reg_executing()~='' or vim.fn.getchar(1)~=0,
      command=mode:sub(1,1)=='c' and vim.fn.getcmdline() or '',save=save,savePath=path,problem=problem}
  end
  return true
end)()
"#;
