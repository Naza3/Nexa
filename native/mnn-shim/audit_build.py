#!/usr/bin/env python3
"""Inventory exact compiled source/flags; fail on direct unaccounted sink.
This is a conservative lexical aid, not a complete C++ reachability proof.
"""
import argparse,json,re,pathlib,hashlib,shlex
p=argparse.ArgumentParser();p.add_argument('build',type=pathlib.Path);p.add_argument('source',type=pathlib.Path);a=p.parse_args()
commands=json.loads((a.build/'compile_commands.json').read_text());entries=[];fail=[]
for file in sorted({x['file'] for x in commands if str(a.source) in x['file']}):
 f=pathlib.Path(file);text=f.read_text(errors='replace');cmds=[x['command'] for x in commands if x['file']==file]
 for cmd in cmds:
  tokens=shlex.split(cmd); definitions=[]; all_definitions=[]
  for index,token in enumerate(tokens):
   if token in ('-D','-U') and index+1<len(tokens):token+=tokens[index+1]
   if token.startswith('-D') or token.startswith('-U'):all_definitions.append(token)
   if token.startswith('-DNEXA_MNN_SILENT_LOGS') or token.startswith('-UNEXA_MNN_SILENT_LOGS'):definitions.append(token)
  if not definitions or any(token!='-DNEXA_MNN_SILENT_LOGS=1' for token in definitions):fail.append('missing or conflicting silent macro '+file)
  for forbidden in ['LLM_LOG_TO_STRING','JINJA_DEBUG','DUMP_PROFILE_INFO','DEBUG_IMAGE','MNN_OPEN_TIME_TRACE','MNN_DUMP_MEMORY','EAGLE_DEBUG']:
   if any(token=='-D'+forbidden or token.startswith('-D'+forbidden+'=') for token in all_definitions):fail.append('forbidden '+forbidden)
 sinks=[]
 for n,line in enumerate(text.splitlines(),1):
  if re.search(r'\b(printf|fprintf|puts|fputs|fwrite|__android_log_print)\s*\(|std::(cout|cerr|clog)',line) and not line.strip().startswith('//'):
   # Explicit compile guards on direct iostream sites in patch 0003.
   guard=(str(f.relative_to(a.source))=='source/core/FileLoader.cpp' and 'fwrite(' in line) or 'NEXA_MNN_SILENT_LOGS' in '\n'.join(text.splitlines()[max(0,n-3):n-1])
   sinks.append({'line':n,'guarded':guard})
   if not guard:fail.append('unclassified direct sink '+file+':'+str(n))
 entries.append({'path':str(f.relative_to(a.source)),'sha256':hashlib.sha256(f.read_bytes()).hexdigest(),'direct_sinks':sinks})
# Linux Make emits dependency files; Ninja records the same inventory in its
# dependency database. Only source-tree headers are classified below.
headers=set()
for dep in a.build.rglob('*.o.d'):
 for item in dep.read_text().replace('\\\n',' ').split():
  f=pathlib.Path(item)
  if str(f).startswith(str(a.source)) and f.suffix in ('.h','.hpp'):headers.add(f)
if not headers and (a.build/'build.ninja').exists():
 import subprocess
 make_program=next(line.split('=',1)[1] for line in (a.build/'CMakeCache.txt').read_text().splitlines() if line.startswith('CMAKE_MAKE_PROGRAM:'))
 output=subprocess.check_output([make_program,'-C',str(a.build),'-t','deps'],text=True)
 for line in output.splitlines():
  f=pathlib.Path(line.strip())
  if str(f).startswith(str(a.source)) and f.suffix in ('.h','.hpp'):headers.add(f)
header_entries=[]
for f in sorted(headers):
 content=f.read_text(errors='replace'); stripped=re.sub(r'/\*.*?\*/',lambda m:'\n'*m[0].count('\n'),content,flags=re.S)
 hits=[]
 for n,line in enumerate(stripped.splitlines(),1):
  if not re.search(r'\b(printf|fprintf|puts|fputs|fwrite|__android_log_print)\s*\(|std::(cout|cerr|clog)',line) or line.strip().startswith('//'):continue
  path=str(f.relative_to(a.source));reason=None
  if path=='include/MNN/MNNDefine.h':reason='inactive branches; highest-priority silent define is required on every compiled unit'
  elif path in ('transformers/llm/engine/include/llm/llm.hpp','transformers/llm/engine/src/omni.hpp') and '&std::cout' in line:reason='default ostream argument only; shim never calls response and always uses nullptr generate_init'
  elif path=='transformers/llm/engine/src/speculative_decoding/tokentree.hpp':reason='literal #if 0 dump block'
  elif path=='transformers/llm/engine/src/tokenizer/jinja.hpp':reason='JINJA_DEBUG compile-time forbidden and undefined'
  hits.append({'line':n,'reason':reason})
  if not reason:fail.append('unclassified header sink '+path+':'+str(n))
 header_entries.append({'path':str(f.relative_to(a.source)),'sha256':hashlib.sha256(f.read_bytes()).hexdigest(),'direct_sinks':hits})
if not headers:fail.append('missing transitive-header dependency evidence')
report={'compiled_source_count':len(entries),'sources':entries,'header_count':len(header_entries),'headers':header_entries,'failures':fail,'limitations':['Compiled source and actual dependency-header lexical scan has explicit classified sinks; external system libraries and native crash diagnostics remain outside this application logging boundary.','Runtime canaries and explicit file-dump reachability review remain required; Android logcat requires a device.']}
(a.build/'logging-audit.json').write_text(json.dumps(report,indent=2)+'\n')
print(json.dumps({'compiled_source_count':len(entries),'header_count':len(header_entries),'failures':fail}));raise SystemExit(bool(fail))
