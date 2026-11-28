from pathlib import Path
import collections,hashlib,json,subprocess
root=Path.cwd()
r=json.loads(Path('log/fredfix-round18-probe-results.json').read_text())
assert len(r)==141 and all(x['pass'] for x in r)
cases=[]
for x in r:
 item={k:x[k] for k in ['probe','variant','file','fixture_sha256','expected','exit','pass']}
 item['diagnostics']=x['output'].strip().splitlines()[:5]
 for k in ['compile_or_test_command','compile_or_test_exit']:
  if k in x:item[k]=x[k]
 if x.get('compile_or_test_exit',0)!=0:
  item['sensitivity_evidence']=[line for line in x['compile_or_test_output'].splitlines() if 'REL-DANGLING' in line or 'test result:' in line or 'assertion' in line or 'round18_discovery_loss_polarity' in line][-8:]
 cases.append(item)
summary={'variants':len(r),'all_passed':True,'positive_controls':sum(x['expected']=='accept' for x in r),'gate_negative_variants':sum(x['expected']=='reject' for x in r),'cases':cases}
Path('log/fredfix-round18-probe-evidence.json').write_text(json.dumps(summary,indent=2)+'\n')
gates=json.loads(Path('log/fredfix-round18-final-gate-summary.json').read_text())
assert len(gates)==20 and all(g['accepted'] for g in gates.values())
assert gates['workspace-tests']['baseline_exact_match'] and len(gates['workspace-tests']['failures'])==19
assert gates['contracts']['mutations']==['140'] and gates['contracts']['rows']==['197']
source_hashes=json.loads(Path('log/fredfix-round18-final-source-hashes.json').read_text())
assert source_hashes['unchanged_after_gates']
assert {k:hashlib.sha256((root/k).read_bytes()).hexdigest() for k in source_hashes['hashes']}==source_hashes['hashes']
current={str(p.relative_to(root)) for p in (root/'crates/validate').rglob('*') if not any(x in p.parts for x in ['target','__pycache__','.git']) and p.is_file()}
current|={'Cargo.toml','Cargo.lock','ROADMAP.md','docs/gaps.md','docs/roadmap.md','docs/file-format-parity.md','apps/wordcraft-cli/src/main.rs','apps/wordcraft-cli/src/tests.rs','xtask/src/layers.rs'}
assert current==set(source_hashes['hashes']),'input set changed after gates'
patch=subprocess.check_output(['git','diff','--binary','8dc309af','--','.',':(exclude)log/**'])
Path('log/fredfix-round18-reviewed.diff').write_bytes(patch)
Path('log/fredfix-round18-reviewed-diff.json').write_text(json.dumps({'base':'8dc309afe132bb09e90e7b19faf99cbfaf82db81','implementation_diff':'log/fredfix-round18-reviewed.diff','sha256':hashlib.sha256(patch).hexdigest(),'scope':'implementation and progress documents; log evidence excluded','no_dependency_changes':not subprocess.check_output(['git','diff','8dc309af','--','Cargo.toml','Cargo.lock','crates/validate/Cargo.toml','crates/validate/observations/recognizer/Cargo.toml','crates/validate/observations/recognizer/Cargo.lock'])},indent=2)+'\n')
print('source variants',len(r),'all passed; final gate accepted; reviewed implementation diff',hashlib.sha256(patch).hexdigest())
