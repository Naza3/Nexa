#!/usr/bin/env python3
"""Verify exact upstream and patch pre/postimages; apply only to a private copy."""
import argparse, hashlib, json, pathlib, subprocess, shutil
HERE=pathlib.Path(__file__).resolve().parent
COMMIT='d407447ed56c4121a11ccbd266dc184ca1ead0c2'
def sha(p): return hashlib.sha256(p.read_bytes()).hexdigest()
def run(*args): return subprocess.check_output(args,text=True).strip()
def verify(root, files, key):
 for item in files:
  if (root/item['path']).is_symlink(): raise SystemExit('patch target must be a regular private file')
  if sha(root/item['path']) != item[key]: raise SystemExit('source identity mismatch: '+item['path'])
def main():
 ap=argparse.ArgumentParser(); ap.add_argument('--source',type=pathlib.Path,required=True); ap.add_argument('--destination',type=pathlib.Path); ap.add_argument('--verify-post',action='store_true'); a=ap.parse_args()
 lock=json.loads((HERE/'lock.json').read_text())
 for patch in lock['patches']:
  if sha(HERE/patch['file']) != patch['sha256']: raise SystemExit('patch identity mismatch')
 if a.verify_post:
  if run('git','-C',str(a.source),'rev-parse','HEAD') != COMMIT: raise SystemExit('upstream mismatch')
  changed=set(run('git','-C',str(a.source),'diff','--name-only','HEAD').splitlines())
  if changed != {x['path'] for x in lock['files']}: raise SystemExit('unexpected private source modifications')
  if run('git','-C',str(a.source),'ls-files','--others','--exclude-standard'): raise SystemExit('unexpected private source files')
  verify(a.source,lock['files'],'after'); return
 if run('git','-C',str(a.source),'rev-parse','HEAD') != COMMIT: raise SystemExit('upstream mismatch')
 if run('git','-C',str(a.source),'status','--porcelain','--untracked-files=all'): raise SystemExit('upstream must be clean')
 verify(a.source,lock['files'],'before')
 if not a.destination or a.destination.exists(): raise SystemExit('destination must be new private directory')
 shutil.copytree(a.source,a.destination,symlinks=True)
 for patch in lock['patches']:
  file=str(HERE/patch['file']); run('git','-C',str(a.destination),'apply','--check',file); run('git','-C',str(a.destination),'apply','--whitespace=error',file)
 verify(a.destination,lock['files'],'after')
 print(lock['patch_set_sha256'])
if __name__=='__main__': main()
