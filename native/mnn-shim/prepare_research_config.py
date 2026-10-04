#!/usr/bin/env python3
"""Create only a fixed, hash-verified research runtime config; not model import."""
import argparse,hashlib,json,pathlib
p=argparse.ArgumentParser();p.add_argument('--model-root',type=pathlib.Path,required=True);p.add_argument('--output',type=pathlib.Path,required=True);p.add_argument('--threads',type=int,default=2);p.add_argument('--chunk',type=int,default=32);p.add_argument('--context',type=int,default=2048);p.add_argument('--max-tokens',type=int,default=12);a=p.parse_args()
def require(condition,message):
 if not condition:raise SystemExit(message)
require(1<=a.threads<=2 and 1<=a.chunk<=128 and 1<=a.context<=2048 and 1<=a.max_tokens<=a.context,'invalid research policy')
root=a.model_root.resolve();output=a.output.resolve();require(not output.is_relative_to(root),'runtime config must be outside immutable candidate')
lock=json.loads((pathlib.Path(__file__).resolve().parents[2]/'scripts/android_mnn/candidate-model.json').read_text())
for name,spec in lock['files'].items():
 f=root/name;require(f.is_file() and not f.is_symlink() and f.stat().st_size==spec['size'],'candidate size/type mismatch: '+name)
 h=hashlib.sha256()
 with f.open('rb') as stream:
  for block in iter(lambda:stream.read(1024*1024),b''):h.update(block)
 require(h.hexdigest()==spec['sha256'],'candidate hash mismatch: '+name)
config=json.loads((root/'llm_config.json').read_text());require(hashlib.sha256(config['jinja']['chat_template'].encode()).hexdigest()==lock['template_sha256'],'candidate template mismatch')
config.update(base_dir=str(root)+'/',backend_type='cpu',thread_num=a.threads,precision='high',memory='low',sampler_type='greedy',chunk=a.chunk,max_all_tokens=a.context,max_new_tokens=a.max_tokens,reuse_kv=False,prompt_cache=False,use_mmap=False,use_cached_mmap=False,kvcache_mmap=False,speculative_type='',context_file='nexa-no-context.json')
config['async']=False;config['jinja']['context']={'enable_thinking':False}
output.parent.mkdir(parents=True,exist_ok=True);output.write_text(json.dumps(config,ensure_ascii=False,separators=(',',':'))+'\n')
print(json.dumps({'fixed_research_candidate_verified':True,'file_count':len(lock['files']),'template_sha256':lock['template_sha256'],'threads':a.threads,'chunk':a.chunk,'context':a.context,'production_admitted':False}))
