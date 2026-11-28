from pathlib import Path
import hashlib,json,os,re,subprocess
root=Path.cwd()
prefix="fredfix-round18-final"
gates=[['recognizer-fmt', ['cargo', 'fmt', '--manifest-path', 'crates/validate/observations/recognizer/Cargo.toml', '--check'], {}], ['recognizer-clippy', ['cargo', 'clippy', '--locked', '--manifest-path', 'crates/validate/observations/recognizer/Cargo.toml', '--target-dir', 'target/observation-recognizer', '--all-targets', '--', '-D', 'warnings'], {}], ['fmt', ['cargo', 'fmt', '--all', '--check'], {}], ['clippy', ['cargo', 'clippy', '-p', 'wordcraft-validate', '--all-targets', '--', '-D', 'warnings'], {}], ['tests', ['cargo', 'test', '-p', 'wordcraft-validate'], {}], ['corpus', ['cargo', 'test', '-p', 'wordcraft-validate'], {'WORDCRAFT_VALIDATE_CORPUS': 'log/fredapps-stress-corpus'}], ['cli', ['cargo', 'test', '-p', 'wordcraft-cli', '--bin', 'wordcraft-cli'], {}], ['doc-coverage', ['cargo', 'rustdoc', '-p', 'wordcraft-validate', '--lib', '--', '-Z', 'unstable-options', '--show-coverage', '--document-private-items'], {'RUSTC_BOOTSTRAP': '1'}], ['contracts', ['python3', 'crates/validate/observations/generate.py', '--check', '--self-test'], {}], ['model', ['python3', 'crates/validate/model/verify_model.py'], {}], ['cli-build', ['cargo', 'build', '-p', 'wordcraft-cli', '--bin', 'wordcraft-cli'], {}], ['workspace-ci', ['cargo', 'xtask', 'ci'], {}], ['workspace-tests', ['cargo', 'test', '--workspace', '--no-fail-fast'], {}], ['i18n-report', ['cargo', 'xtask', 'i18n-report'], {}], ['language', ['cargo', 'xtask', 'language'], {}], ['doctest', ['cargo', 'test', '--workspace', '--doc'], {}], ['assets', ['cargo', 'xtask', 'assets'], {}], ['layers', ['cargo', 'xtask', 'layers'], {}], ['wasm', ['cargo', 'xtask', 'wasm'], {}], ['diff-check', ['git', 'diff', '--check', 'HEAD'], {}]]
def inputs():
 paths=[p for p in (root/'crates/validate').rglob('*') if not any(x in p.parts for x in ['target','__pycache__','.git']) and p.is_file()]
 paths += [root/p for p in ['Cargo.toml','Cargo.lock','ROADMAP.md','docs/gaps.md','docs/roadmap.md','docs/file-format-parity.md','apps/wordcraft-cli/src/main.rs','apps/wordcraft-cli/src/tests.rs','xtask/src/layers.rs']]
 return {str(p.relative_to(root)):hashlib.sha256(p.read_bytes()).hexdigest() for p in paths}
before=inputs()
versions={}
for cmd in [['cargo','--version'],['rustc','--version'],['python3','--version']]:
 run=subprocess.run(cmd,text=True,capture_output=True);versions[cmd[0]]={'exit':run.returncode,'output':run.stdout+run.stderr}
Path(f'log/{prefix}-tool-versions.json').write_text(json.dumps(versions,indent=2)+'\n')
Path(f'log/{prefix}-gate-commands.json').write_text(json.dumps(gates,indent=2)+'\n')
summary={}
env=os.environ.copy();env.pop('WORDCRAFT_VALIDATE_CORPUS',None)
for name,cmd,extra in gates:
 print('starting',name,flush=True)
 e={**env,**extra}
 if name=='corpus':e['WORDCRAFT_VALIDATE_CORPUS']=str(root/'log/fredapps-stress-corpus')
 out=Path(f'log/{prefix}-{name}.log')
 with out.open('w') as log: code=subprocess.run(cmd,env=e,stdout=log,stderr=subprocess.STDOUT).returncode
 content=out.read_text(errors='replace')
 counts=[tuple(map(int,m)) for m in re.findall(r'test result:.*?(\d+) passed; (\d+) failed; (\d+) ignored',content)]
 item={'exit':code,'counts':dict(zip(['passed','failed','ignored'],[sum(x[i] for x in counts) for i in range(3)])),'summaries':len(counts),'accepted':code==0}
 if name in {'workspace-tests','workspace-ci'}:
  fail_lines=re.findall(r'^test (.*?) \.\.\. FAILED$',content,re.M)
  failures=sorted(set(fail_lines))
  failed_summaries=len(re.findall(r'^test result: FAILED\.',content,re.M))
  unclassified_failure=bool(re.search(r"process didn't exit successfully:|could not compile|could not execute process",content))
  baseline=json.loads(Path('log/fredfix-round6-baseline-failures.json').read_text())['failures'] if name=='workspace-tests' else json.loads(Path('log/fredfix-round17-ci-baseline.json').read_text())['baseline']
  item.update(failures=failures,baseline_exact_match=failures==sorted(baseline),new_blockers=sorted(set(failures)-set(baseline)),missing_baseline_failures=sorted(set(baseline)-set(failures)))
  item['accepted']=item['baseline_exact_match'] and code in {1,101} and item['counts']['failed']>0 and len(fail_lines)==item['counts']['failed'] and failed_summaries>0 and not unclassified_failure
 if name=='contracts':
  item['mutations']=re.findall(r'(\d+) source mutations rejected',content)
  item['rows']=re.findall(r'(\d+) declared bounded consumers',content)
 if name in {'tests','corpus'}: item['accepted'] &= item['counts']==dict(passed=508,failed=0,ignored=0)
 summary[name]=item
 Path(f'log/{prefix}-gate-summary.json').write_text(json.dumps(summary,indent=2)+'\n')
 print(name,item,flush=True)
 assert inputs()==before,'gate changed an implementation input'
Path(f'log/{prefix}-source-hashes.json').write_text(json.dumps({'hashes':before,'unchanged_after_gates':inputs()==before},indent=2)+'\n')
assert all(x['accepted'] for x in summary.values()),summary
print('all final gate criteria satisfied on unchanged implementation inputs',flush=True)
