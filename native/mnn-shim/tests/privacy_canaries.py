#!/usr/bin/env python3
"""Run in isolated child processes; capture logs only in the test harness.
No library stdout/stderr redirection, no persistent log callback, no user data.
"""
import argparse,pathlib,json,subprocess,tempfile,shutil
p=argparse.ArgumentParser();p.add_argument('binary');p.add_argument('config',type=pathlib.Path);a=p.parse_args();base=json.loads(a.config.read_text());reports=[]
with tempfile.TemporaryDirectory(prefix='NEXA_PATH_CANARY_') as directory:
 root=pathlib.Path(directory)
 model=pathlib.Path(base['base_dir'])
 for name in ('llm_config.json','tokenizer.txt'):shutil.copyfile(model/name,root/name)
 for name in ('llm.mnn','llm.mnn.weight'):(root/name).symlink_to(model/name)
 (root/'bad-tokenizer.txt').write_text('0 NEXA_CORRUPT_TOKENIZER_CANARY\n')
 (root/'bad-graph.mnn').write_bytes(b'NEXA_CORRUPT_GRAPH_CANARY')
 cases={'corrupt_config':{}, 'corrupt_tokenizer':{'base_dir':str(root)+'/', 'tokenizer_file':'bad-tokenizer.txt'}, 'corrupt_graph':{'base_dir':str(root)+'/', 'llm_model':'bad-graph.mnn'}, 'bad_template':{'jinja':{'chat_template':'{% NEXA_TEMPLATE_CANARY_INVALID %}', 'eos':'<|im_end|>', 'context':{'enable_thinking':False}}}, 'missing_tokenizer':{'tokenizer_file':'NEXA_TOKENIZER_CANARY_DOES_NOT_EXIST'},'missing_graph':{'llm_model':'NEXA_GRAPH_CANARY_DOES_NOT_EXIST'},'unsafe_backend':{'backend_type':'NEXA_BACKEND_CANARY'},'unsafe_option':{'ple_embed_file':'NEXA_AUXILIARY_CANARY'}}
 for name,change in cases.items():
  config=root/(name+'.json');config.write_text('{ NEXA_CONFIG_CANARY' if name=='corrupt_config' else json.dumps(base|change));proc=subprocess.run([a.binary,str(config),'prepare-only' if name=='bad_template' else 'load-only'],capture_output=True,text=True,timeout=90)
  expected=['abi_sampler_pass','host_parallel_marker'];lines=proc.stdout.splitlines()
  ok=proc.returncode==0 and proc.stderr=='' and lines[:2]==expected and len(lines)==3 and lines[2] in ('load_result 1','load_result 6') and 'CANARY' not in proc.stdout
  reports.append({'case':name,'passed':ok,'exit_code':proc.returncode})
  if not ok:raise SystemExit(json.dumps(reports))
with tempfile.TemporaryDirectory(prefix='NEXA_SUCCESS_PATH_CANARY_') as directory:
 config=pathlib.Path(directory)/'config.json';config.write_text(json.dumps(base))
 proc=subprocess.run([a.binary,str(config),'privacy-success'],capture_output=True,text=True,timeout=120)
 import re
 lines=proc.stdout.splitlines()
 ok=proc.returncode==0 and proc.stderr=='' and len(lines)==3 and lines[0]=='abi_sampler_pass' and re.fullmatch(r'cancel_phase 4 safe_return_ms [0-9.]+',lines[1]) and lines[2]=='privacy_success_matrix_pass' and 'CANARY' not in proc.stdout
 reports.append({'case':'success_en_zh_multi_phase_cancel_callback_recovery','passed':bool(ok),'exit_code':proc.returncode})
 if not ok:raise SystemExit(json.dumps(reports)+repr(proc.stderr))
print(json.dumps({'privacy_canaries':reports}))
