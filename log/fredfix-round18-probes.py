#!/usr/bin/env python3
"""Executed R18 fixtures: real copied generator --check, with optional isolated crate checks.
Run only on a stable tree; each mutation is asserted and restored in the disposable copy.
"""
from pathlib import Path
import json,os,re,shutil,subprocess,sys,tempfile,tomllib,hashlib
ROOT=Path(__file__).resolve().parents[1]
selected=set(sys.argv[1:])
RESULTS=[]
TMP=Path(tempfile.mkdtemp(prefix='wordcraft-r18-'))
COPY=TMP/'crates/validate'
shutil.copytree(ROOT/'crates/validate',COPY,ignore=shutil.ignore_patterns('__pycache__','target'))
(ROOT/'target/r18-probe-tool').mkdir(parents=True, exist_ok=True)
(TMP/'target').symlink_to(ROOT/'target/r18-probe-tool',target_is_directory=True)
# A complete isolated validator crate, retaining all original sources/tests and locked versions.
manifest=(ROOT/'Cargo.toml').read_text()
lock=tomllib.loads((ROOT/'Cargo.lock').read_text())
versions={p['name']:p['version'] for p in lock['package'] if p['name'] in {'zip','quick-xml','serde','serde_json'}}
root_manifest='''[workspace]
resolver = "3"
members = ["crates/validate"]
[workspace.package]
version = "0.15.1"
edition = "2024"
license = "MIT OR Apache-2.0"
[workspace.dependencies]
'''
for n in versions:
 line=next(x for x in manifest.splitlines() if x.startswith(n+' = '))
 root_manifest+=line+'\n'
root_manifest+=manifest[manifest.index('[workspace.lints.rust]'):manifest.index('[profile.dev.package.')]
(TMP/'Cargo.toml').write_text(root_manifest)
shutil.copyfile(ROOT/'Cargo.lock',TMP/'Cargo.lock')
SRC=COPY/'src';GEN=COPY/'observations/generate.py'
originals={p.name:p.read_text() for p in SRC.glob('*.rs')}
original_outputs={p:p.read_bytes() for p in [COPY/'observation-contracts.md',SRC/'contracts_generated.rs']}
ctx={'__name__':'delta_fixtures'}
fixture=(ROOT/'log/delta17-probe.py').read_text().replace("ROOT = Path('/tmp/opencode/delta17-review')",f'ROOT = Path({str(TMP)!r})')
exec(compile(fixture,'log/delta17-probe.py','exec'),ctx)
# Map every delta-17 verbatim fixture to its binding red-team number.
labels={
1:'escape/remove-entry-truncation-absence-guard',2:'escape/raw-iterator-absence',3:'escape/helper-mediated-package-absence',4:'escape/existing-canonical-iterator-absence',
5:'escape/aliased-ufcs-observed-absence',6:'escape/self-ufcs-observed-absence',7:'escape/serde-content-types-consumer',8:'false-rejection/ordinary-named-field-binding',
9:'false-rejection/qualified-format',10:'false-rejection/inline-attribute',11:'false-rejection/trait-default-nonconsumer',12:'history/closure-parameter-destructure',13:'history/destructuring-assignment',14:'history/parenthesized-destructuring-assignment',15:'history/macro-produced-pattern',16:'history/raw-field',17:'history/raw-registry-method',18:'history/module-callback',19:'seam/type-alias-carrying-callback',20:'seam/trait-default-body',21:'seam/associated-const-callback',22:'seam/include-item',23:'seam/cfg-gated-consumer',24:'seam/other-macro-path',25:'seam/supported-macro-arguments-audited'}
def command(args,env=None):
 return subprocess.run(args,cwd=TMP,env={**os.environ,**(env or {})},capture_output=True,text=True,timeout=300)
def probe(number,label,name,mutation,accept=False,compile=False,test=None,diagnostic=None,test_fail=False):
 if selected and str(number) not in selected:return
 path=SRC/name
 before=path.read_text();changed=mutation(before)
 assert changed!=before,(number,label,'fixture did not land')
 changes = changed if isinstance(changed, dict) else {name:changed}
 saved = {n:(SRC/n).read_text() for n in changes}
 assert all(changes[n]!=saved[n] for n in changes), 'composite fixture did not land'
 for n,value in changes.items(): (SRC/n).write_text(value)
 record={'probe':number,'variant':label,'file':name,'fixture_sha256':hashlib.sha256(json.dumps(changes,sort_keys=True).encode()).hexdigest(),'expected':'accept' if accept else 'reject'}
 try:
  run=command(['python3',str(GEN),'--check']);record.update(exit=run.returncode,output=run.stdout+run.stderr)
  record['pass']=(run.returncode==0)==accept
  if diagnostic:
   record['diagnostic_expected']=diagnostic;record['pass'] &= diagnostic in record['output']
  if compile or test:
   args=['cargo','test','--offline','--lib',test] if test else ['cargo','check','--offline']
   check=command(args,{'CARGO_TARGET_DIR':str(ROOT/'target/r18-probe-compile')})
   record.update(compile_or_test_command=args,compile_or_test_exit=check.returncode,compile_or_test_output=check.stdout+check.stderr)
   record['pass'] &= (check.returncode!=0 and (test_fail if isinstance(test_fail,str) else 'REL-DANGLING') in record['compile_or_test_output'] and 'test result: FAILED' in record['compile_or_test_output']) if test_fail else check.returncode==0
  assert all(p.read_bytes()==b for p,b in original_outputs.items()),'probe altered register'
 finally:
  for n,value in saved.items(): (SRC/n).write_text(value)
 RESULTS.append(record)
 print(number,label,'PASS' if record['pass'] else 'FAIL',record['exit'],record['output'].splitlines()[0] if record['output'] else '',flush=True)
 (ROOT/'log/fredfix-round18-probe-results.json').write_text(json.dumps(RESULTS,indent=2)+'\n')
def condition(s,new):
 result,n=re.subn(r'if match package::canonical_entry\(pkg, target\) \{.*?\n            \} \{',new,s,count=1,flags=re.S)
 assert n==1
 return result
try:
 for n,label in labels.items():
  _,name,addition=next(p for p in ctx['PROBES'] if p[0]==label)
  if n==1:mutation=lambda s:condition(s,'if package::canonical_entry(pkg, target).is_none() {')
  else:mutation=lambda s,l=label,f=name,a=addition:ctx['mutate'](l,f,a)
  probe(n,label,name,mutation,accept=n in {4,9,10},compile=n in {4,8,9,10},test='round5_archive_prefix_absence' if n==4 else None)
 label='seam/trait-impl-body'
 _,name,addition=next(p for p in ctx['PROBES'] if p[0]==label)
 probe(20,label,name,lambda s:ctx['mutate'](label,name,addition))
 for pat,label in [('Resolution::Absent | Resolution::Unknown(_) => true,','or-pattern'),('_ => true,','wildcard')]:
  body='''if match package::canonical_entry(pkg, target) {
                Resolution::One(_) => false,
                PAT
                Resolution::Ambiguous(_) => false,
            } {'''.replace('PAT',pat)
  probe(26,label,'lib.rs',lambda s,b=body:condition(s,b),diagnostic='absence polarity',test='round5_archive_prefix_absence' if label=='or-pattern' else None,test_fail=label=='or-pattern')
 for label,new in [
  ('matches collapse','if matches!(package::canonical_entry(pkg, target), Resolution::Absent | Resolution::Unknown(_)) {'),
  ('if-let collapse','if let Resolution::Absent | Resolution::Unknown(_) = package::canonical_entry(pkg, target) {'),
  ('block-bound result collapse','if match { let outcome = package::canonical_entry(pkg, target); outcome } { Resolution::One(_) => false, Resolution::Absent | Resolution::Unknown(_) => true, Resolution::Ambiguous(_) => false } {'),
  ('local matches else collapse','if { let outcome = package::canonical_entry(pkg, target); if matches!(outcome, Resolution::One(_)) { false } else { true } } {'),
  ('guard collapse','if match package::canonical_entry(pkg, target) { Resolution::One(_) => false, value if matches!(value, Resolution::Absent | Resolution::Unknown(_)) => true, _ => false } {')]:
  probe(26,label,'lib.rs',lambda s,n=new:condition(s,n),diagnostic='absence polarity')
 body='''if match package::canonical_entry(pkg, target) {
                Resolution::One(_) => false,
                Resolution::Absent => true,
                Resolution::Ambiguous(_) => false,
            } {'''
 probe(27,'missing Unknown','lib.rs',lambda s:condition(s,body),diagnostic='must explicitly cover')
 probe(28,'fabricated Absent','package.rs',lambda s:s.replace('Resolution::discovered(canonical(&pkg.entries, name), !pkg.entries_truncated && !pkg.identity_incomplete)','Resolution::Absent',1),diagnostic='literal Resolution::Absent')
 probe(28,'literal true completeness','package.rs',lambda s:s.replace('Resolution::discovered(canonical(&pkg.entries, name), !pkg.entries_truncated && !pkg.identity_incomplete)','Resolution::discovered(canonical(&pkg.entries, name), true)',1),diagnostic='completeness')
 contract='observation_contract! { rules: "-"; collections: "field.entries*1"; scope: "x"; loss: "x"; fallback: "x"; package_loss: "independent"; }'
 probe(29,'contracted raw absence','package.rs',lambda s:s+'\nfn unregistered_absence(entries: &[Entry], name: &str) -> bool { '+contract+' !entries.iter().any(|entry| entry.name == name) }',diagnostic='an observation contract cannot establish')
 def wired_raw(s):
  return {'package.rs':s+'\npub(crate) fn unregistered_absence(entries: &[Entry], name: &str) -> bool { '+contract+' !entries.iter().any(|entry| entry.name == name) }', 'lib.rs':condition(originals['lib.rs'],'if package::unregistered_absence(&pkg.entries, target) {')}
 probe(29,'contracted raw absence wired to REL-DANGLING','package.rs',wired_raw,diagnostic='raw absence',test='round5_archive_prefix_absence',test_fail=True)
 for ty,param,op in [('type HiddenEntries = [Entry];','entries: &HiddenEntries','!entries.iter().any(|entry| entry.name == name)'),('type HiddenMap = HashMap<String, String>;','entries: &HiddenMap','entries.get(name).is_none()'),('type HiddenPackage = Package;','entries: &HiddenPackage','!entries.entries.iter().any(|entry| entry.name == name)')]:
  probe(30,ty,'package.rs',lambda s,t=ty,p=param,o=op:s+'\n'+t+' fn unregistered_absence('+p+', name: &str) -> bool { '+o+' }',diagnostic='raw absence')
 for helper in ['canonical_entry','entry_by_name','content_unavailable']:
  for binding in ['hidden_lookup','package::'+helper,'hidden_package::'+helper]:
   direct=binding=='hidden_lookup'
   addition=f'\nuse package::{helper} as hidden_lookup; use package as hidden_package; fn unregistered_absence(pkg: &Package, name: &str) -> bool {{ '+('let _ = hidden_lookup(pkg, name);' if direct else f'let lookup = {binding}; let _ = lookup(pkg, name);')+' false }'
   probe(31,helper+'/'+binding,'lib.rs',lambda s,a=addition:s+a,diagnostic='lookup.'+helper)
 for old,new,label in [
  ('pkg.entries_truncated || pkg.identity_incomplete','pkg.identity_incomplete','2751 delete'),
  ('pkg.entries_truncated || pkg.identity_incomplete','!pkg.entries_truncated || pkg.identity_incomplete','2751 negate'),
  ('!pkg.entries_truncated\n        && !pkg.identity_incomplete','!pkg.identity_incomplete','2777 delete'),
  ('!pkg.entries_truncated\n        && !pkg.identity_incomplete','pkg.entries_truncated\n        && !pkg.identity_incomplete','2777 negate')]:
  probe(32,label,'lib.rs',lambda s,o=old,n=new:s.replace(o,n,1),diagnostic='entry-loss',test='round18_discovery_loss_polarity',test_fail='round18_discovery_loss_polarity')
 for callable in ['Self::prefix','HiddenDefinitions::prefix']:
  a='\ntype HiddenDefinitions = Observed<HashSet<String>>; impl Observed<HashSet<String>> { fn unregistered_absence(&self, key: &String) -> bool { let prefix = '+callable+'; !prefix(self).contains(key) } }'
  probe(33,callable,'lib.rs',lambda s,a=a:s+a,diagnostic='CompleteSet')
 old='self.parts.values().flat_map(|p| p.special_notes.prefix().iter().cloned()).collect()'
 for negative in [True,False]:
  body='self.parts.values().flat_map(|p| { let retained = p.special_notes.prefix(); retained.iter().filter(|candidate| '+('!' if negative else '')+'retained.contains(candidate)).cloned() }).collect()'
  probe(34,'exact prefix-only contract, negative='+str(negative),'lib.rs',lambda s,b=body:s.replace(old,b,1),accept=not negative,compile=not negative,diagnostic='CompleteSet' if negative else None)
 for expression in ['Self::prefix(self).contains(key) == false','match Self::prefix(self).contains(key) { false => true, true => false }','matches!(Self::prefix(self).contains(key), false)']:
  a='\nimpl Observed<HashSet<String>> { fn unregistered_absence(&self, key: &String) -> bool { '+expression+' } }'
  probe(35,expression,'lib.rs',lambda s,a=a:s+a,diagnostic='CompleteSet')
 for ty,expression in [('HashSet<String>','!Self::prefix(self).iter().any(|candidate| candidate == key)'),('HashMap<String, String>','Self::prefix(self).get(key).is_none()')]:
  a='\nimpl Observed<'+ty+'> { fn unregistered_absence(&self, key: &String) -> bool { '+expression+' } }'
  probe(36,expression,'lib.rs',lambda s,a=a:s+a,diagnostic='CompleteSet')
 for prefix in ['<Self>::prefix(self)','<Observed<HashSet<String>>>::prefix(self)']:
  a='\nimpl Observed<HashSet<String>> { fn unregistered_absence(&self, key: &String) -> bool { !'+prefix+'.contains(key) } }'
  probe(37,'negative/'+prefix,'lib.rs',lambda s,a=a:s+a,diagnostic='CompleteSet')
 for expression in ['Observed::is_complete(&self)','Self::is_complete(&self)','HiddenObservation::is_complete(&self)','<Self>::is_complete(&self)','<Observed<HashSet<K>>>::is_complete(&self)','{ let test = Self::is_complete; test(&self) }']:
  probe(37,'CompleteSet provider/'+expression,'observed.rs',lambda s,e=expression:s.replace('if self.is_complete() {','if '+e+' {',1)+'\ntype HiddenObservation<T> = Observed<T>;\n',accept=True,test='observed::tests')
 for derive in ['serde::Serialize','RegistrySerialize']:
  addition='\nfn unregistered_registry_serde<S: serde::Serializer>(registry: &ContentTypes, serializer: S) -> Result<S::Ok, S::Error> { serde::Serialize::serialize(registry, serializer) }'
  if derive=='RegistrySerialize':addition+='\nuse serde::Serialize as RegistrySerialize;'
  probe(38 if derive=='serde::Serialize' else 39,derive,'package.rs',lambda s,d=derive,a=addition:s.replace('#[derive(Default)]\npub(crate) struct ContentTypes {','#[derive(Default, '+d+')]\npub(crate) struct ContentTypes {',1)+a,diagnostic='unauthorized reader derive serde::Serialize')
 for derive in ['Debug','serde::Deserialize']:
  addition='\nfn unregistered_registry_debug(registry: &ContentTypes) -> String { std::format!("{registry:?}") }' if derive=='Debug' else ''
  probe(40,derive,'package.rs',lambda s,d=derive,a=addition:s.replace('#[derive(Default)]\npub(crate) struct ContentTypes {','#[derive(Default, '+d+')]\npub(crate) struct ContentTypes {',1)+a,diagnostic='unauthorized reader derive')
 policy=json.loads((COPY/'observations/reader-contracts.json').read_text())
 for file in sorted({r['type'].split('::')[0] for r in policy['readers']}):
  def qualified(s):
   def attrs(m):
    return '#[derive('+re.sub(r'(?<![\w:])Debug(?!\w)','std::fmt::Debug',re.sub(r'(?<![\w:])Serialize(?!\w)','serde::Serialize',m[1]))+')]'
   return re.sub(r'#\[derive\(([^)]*)\)\]',attrs,s)
  probe(41,'all existing reader pairs in '+file,file,qualified,accept=True,compile=True)
 for type_id in sorted({r['type'] for r in policy['readers']}):
  file,qualified_type=type_id.split('::',1)
  ty=qualified_type.rsplit('::',1)[-1]
  source_ty=re.escape(ty)
  tpl=re.search(r'<template ([^>]+)>',qualified_type)
  if tpl:
   macro_name=tpl.group(1)
   def unauthorized(s,mac=macro_name):
    m=re.search(r'macro_rules!\s+'+re.escape(mac)+r'\s*\{',s)
    assert m,(file,mac)
    body=s[m.end():]
    sub,n=re.subn(r'((?:pub(?:\(crate\))?\s+)?(?:struct|enum)\s+\$name(?=[\s<(;{]))',r'#[derive(serde::Deserialize)] \1',body,count=1)
    assert n==1,(file,mac)
    return s[:m.end()]+sub
  elif re.search(r'(?:struct|enum)\s+'+source_ty+r'(?=[\s<(;{])',originals[file]):
   def unauthorized(s,ty=source_ty):
    value,n=re.subn(r'((?:pub(?:\(crate\))?\s+)?(?:struct|enum)\s+'+ty+r'(?=[\s<(;{]))',r'#[derive(serde::Deserialize)] \1',s,count=1)
    assert n==1,(file,ty)
    return value
  else:
   inst=re.search(r'(\w+)\s*!\s*\(\s*'+source_ty+r'\b',originals[file])
   if not inst:
    raise AssertionError(f'missing literal or macro declaration for {type_id}')
   macro_name=inst.group(1)
   def unauthorized(s,mac=macro_name):
    m=re.search(r'macro_rules!\s+'+re.escape(mac)+r'\s*\{',s)
    assert m,(file,mac)
    body=s[m.end():]
    sub,n=re.subn(r'((?:pub(?:\(crate\))?\s+)?(?:struct|enum)\s+\$name(?=[\s<(;{]))',r'#[derive(serde::Deserialize)] \1',body,count=1)
    assert n==1,(file,mac)
    return s[:m.end()]+sub
  probe(41,'unauthorized pair/'+type_id,file,unauthorized,diagnostic=type_id)
 def nested_reader(s):
  definition=re.search(r'(pub\(crate\) struct Limits \{.*?\n\})',s,re.S)
  assert definition
  s=s.replace('#[derive(Debug, Clone, Copy)]\npub(crate) struct Limits','#[derive(Clone, Copy)]\npub(crate) struct Limits',1)
  return s+'\nfn unregistered_nested_reader() { #[derive(Debug)] '+definition[1]+' }'
 probe(41,'same exact DTO fields transferred into function scope','lib.rs',nested_reader,diagnostic='<function unregistered_nested_reader>::Limits',compile=True)
 probe(41,'same named DTO in another module','package.rs',lambda s:s+'\n#[derive(Debug)] struct Finding { rule: String }',diagnostic='unauthorized reader derive')
 probe(41,'registry field added to allowed DTO','lib.rs',lambda s:s.replace('pub struct Finding {','pub struct Finding { hidden_registry: package::ContentTypes,',1),diagnostic='changed representation/DTO field shape')
 probe(42,'qualified consuming format','lib.rs',lambda s:s+'\nfn unregistered_read(pkg: &Package) -> String { std::format!("{}", pkg.entries.len()) }',diagnostic='field.entries*1')
 for body in ['{ let v = std::vec![1, 2]; v.len() }','{ std::debug_assert!(true); 1 }','{ if std::matches!(true, true) { 1 } else { 0 } }']:
  probe(42,'qualified nonconsuming grammar/'+body,'lib.rs',lambda s,b=body:s+'\nfn nonconsuming() -> usize '+b,accept=True,compile=True)
 impostor='\nmacro_rules! hidden_format { ($package:expr) => { $package.entries.len() }; }\nuse hidden_format as format;\nfn unregistered_read(pkg: &Package) -> usize { crate::format!(pkg) }\n'
 probe(43,'macro name impostor','lib.rs',lambda s:s+impostor,diagnostic='unsupported macro')
 probe(43,'std namespace impostor','lib.rs',lambda s:s+'\nuse external as std;',diagnostic='macro-shadowing import')
 for attribute in ['inline','inline(always)','inline(never)','cold']:
  probe(44,attribute,'lib.rs',lambda s,a=attribute:s+'\n#['+a+'] fn configured_entry_limit(limits: &Limits) -> usize { limits.entries }',accept=True,compile=True)
 for attribute in ['cfg(all())','derive(Debug)','serde(skip)']:
  probe(44,'unauthorized '+attribute,'lib.rs',lambda s,a=attribute:s+'\n#['+a+'] fn unregistered_read(pkg: &Package) -> usize { pkg.entries.len() }')
 probe(24,'new unsupported std env seam','lib.rs',lambda s:s+'\nfn unregistered() { std::env!("X"); }',diagnostic='unrecognized expression/pattern macro')
 # Every remaining package field-read occurrence is independently deleted and negated.
 for index,match in enumerate(re.finditer(r'!pkg\.entries_truncated',originals['package.rs'])):
  for change in ['true','pkg.entries_truncated']:
   probe(32,'package field read '+str(index)+' -> '+change,'package.rs',lambda s,m=match,c=change:s[:m.start()]+c+s[m.end():],diagnostic='entry-loss')
 for old,new,label in [
  ('zip.len() > limits.entries','zip.len() < limits.entries','loader limit inversion'),
  ('if entries_truncated {','if !entries_truncated {','loader informational polarity'),
  ('!content_types_present && !entries_truncated','!content_types_present','loader manifest guard delete'),
  ('!content_types_present && entries_truncated','!content_types_present && !entries_truncated','loader manifest incomplete negate'),
  ('!entries_truncated && !unknown_unreadable','!unknown_unreadable','loader main discovery delete'),
  ('!entries_truncated && !unknown_unreadable','entries_truncated && !unknown_unreadable','loader main discovery negate')]:
  probe(32,label,'package.rs',lambda s,o=old,n=new:s.replace(o,n,1),diagnostic='entry-loss')
 for ty in ['Resolution<String>','HiddenResolution']:
  a='\ntype HiddenResolution = Resolution<String>; fn unregistered_resolution_absence(value: '+ty+') -> bool { match value { Resolution::One(_) => false, Resolution::Absent | Resolution::Unknown(_) => true, Resolution::Ambiguous(_) => false } }'
  probe(26,'parameter collapse/'+ty,'lib.rs',lambda s,a=a:s+a,diagnostic='absence polarity')
 probe(26,'Resolution Self receiver collapse','resolution.rs',lambda s:s+'\nimpl<T> Resolution<T> { fn unregistered_resolution_absence(&self) -> bool { match self { Self::One(_) => false, Self::Absent | Self::Unknown(_) => true, Self::Ambiguous(_) => false } } }',diagnostic='absence polarity')
 probe(26,'opaque result argument collapse','lib.rs',lambda s:condition(s,'if opaque_verdict(package::canonical_entry(pkg, target)) {')+'\nfn opaque_verdict<T>(_value: T) -> bool { true }',diagnostic='absence polarity')
 probe(30,'aliased bool return with exact raw contract','package.rs',lambda s:s+'\ntype HiddenVerdict = (bool); fn unregistered_return_alias(entries: &[Entry], name: &str) -> HiddenVerdict { observation_contract! { rules: "-"; collections: "-"; scope: "raw absence control"; loss: "entry prefix"; fallback: "skip unknown"; package_loss: "independent"; } !entries.iter().any(|entry| entry.name == name) }',diagnostic='raw absence')
 # Later stage probes are installed below this marker.
finally:
 shutil.rmtree(TMP)
failed=[(r['probe'],r['variant']) for r in RESULTS if not r['pass']]
print('failures:',failed,flush=True)
raise SystemExit(bool(failed))
