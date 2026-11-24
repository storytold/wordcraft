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
        (Self { requests, replies, cancelled: Arc::new(AtomicBool::new(false)) }, incoming, outgoing)
    }

    pub fn start(config: String, ctx: egui::Context) -> Result<Self, String> {
        let (requests, incoming) = mpsc::sync_channel(1);
        let (outgoing, replies) = mpsc::channel();
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
                    let result = editor.request(&request);
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
        Ok(Self { requests, replies, cancelled })
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
        editor.request(&serde_json::json!({"initialize":true}))?;
        Ok(editor)
    }

    fn request(&mut self, request: &Value) -> Result<Value, String> {
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
        let deadline = Instant::now() + Duration::from_secs(5);
        let status = loop {
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
  rpc('nvim_exec_lua',code,{})
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
  result=rpc('nvim_exec_lua','return WordCraftModal.snapshot()',{})
end
result.accepted=accepted
io.write(vim.json.encode(result))
"#;

// This is integration code against the public API, not copied editor implementation code.
const INIT: &str = r#"
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
  function m.request(r)
    if vim.api.nvim_get_current_buf()~=m.buf then return false end
    if r.sync then
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
    local cursor=other and (m.cursor or {1,0}) or vim.api.nvim_win_get_cursor(0)
    if not other then m.cursor=cursor end
    local focus=math.min(offset(cursor),#text)
    while focus>0 and text:byte(focus+1) and text:byte(focus+1)>=128 and text:byte(focus+1)<=191 do focus=focus-1 end
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
    return {text=text,edits=edits,focus=focus,anchor=anchor,mode=mode,sequence=sequence,
      command=mode:sub(1,1)=='c' and vim.fn.getcmdline() or '',save=save,savePath=path,problem=problem}
  end
  return true
end)()
"#;
