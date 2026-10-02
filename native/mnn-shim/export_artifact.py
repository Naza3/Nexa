#!/usr/bin/env python3
"""Export only existing verified static archives; no implicit build/download."""
import argparse,pathlib,json,hashlib,subprocess,shutil,re,sys
p=argparse.ArgumentParser();p.add_argument('build',type=pathlib.Path);p.add_argument('--target',required=True);p.add_argument('--compiler',required=True);p.add_argument('--cxx-static',type=pathlib.Path);p.add_argument('--cxxabi-static',type=pathlib.Path);p.add_argument('--unwind-static',type=pathlib.Path);p.add_argument('--builtins-static',type=pathlib.Path);a=p.parse_args()
cache={}
for line in (a.build/'CMakeCache.txt').read_text().splitlines():
 if '=' in line and ':' in line.split('=')[0]:cache[line.split(':')[0]]=line.split('=',1)[1]
if cache.get('CMAKE_BUILD_TYPE')!='Release':raise SystemExit('Release build required')
compiler=cache.get('CMAKE_CXX_COMPILER')
if not compiler:
 compiler_file=next((a.build/'CMakeFiles').glob('*/CMakeCXXCompiler.cmake'))
 compiler=re.search(r'set\(CMAKE_CXX_COMPILER "([^"]+)"',compiler_file.read_text())[1]
actual_compiler=subprocess.check_output([compiler,'--version'],text=True).splitlines()[0]
if actual_compiler!=a.compiler:raise SystemExit('compiler identity mismatch')
root=pathlib.Path(__file__).resolve().parent
subprocess.run([sys.executable,str(root/'../mnn-patches/identity.py'),'--source',cache['NEXA_MNN_SOURCE'],'--verify-post'],check=True)
subprocess.run([sys.executable,str(root/'audit_build.py'),str(a.build),cache['NEXA_MNN_SOURCE']],check=True)
lock=json.loads((root/'../mnn-patches/lock.json').read_text())
def sha(f):return hashlib.sha256(f.read_bytes()).hexdigest()
lib=a.build/'artifact/lib';lib.mkdir(parents=True,exist_ok=True);files=[]
archives=[('nexa-mnn-shim',a.build/'libnexa-mnn-shim.a'),('MNN',a.build/'mnn/libMNN.a')]
if a.cxx_static: archives += [('c++_static',a.cxx_static),('c++abi',a.cxxabi_static),('unwind',a.unwind_static),('clang_rt_builtins',a.builtins_static)]
for name,file in archives:
 if not file or not file.is_file():raise SystemExit('missing archive '+str(file))
 if file.open('rb').read(8)!=b'!<arch>\n':raise SystemExit('expected actual archive: '+str(file))
 dst=lib/('lib'+name+'.a');shutil.copyfile(file,dst);files.append({'name':name,'path':'lib/'+dst.name,'sha256':sha(dst)})
manifest={'schema_version':1,'abi_version':1,'target':a.target,'upstream_commit':lock['upstream_commit'],'patch_set_sha256':lock['patch_set_sha256'],'policy_sha256':lock['policy_sha256'],'header_sha256':sha(root/'include/nexa_mnn.h'),'compiler':a.compiler,'build_type':'Release','static_libraries':files,'system_libraries':['m','dl','pthread','stdc++'] if 'linux-gnu' in a.target else ['m','dl','log','android'],'silent_logs':True}
if 'android' in a.target:
 ndk=pathlib.Path(cache['CMAKE_TOOLCHAIN_FILE']).parents[2]
 props=dict(line.split('=',1) for line in (ndk/'source.properties').read_text().splitlines() if '=' in line)
 revision={k.strip():v.strip() for k,v in props.items()}['Pkg.Revision']
 api=int(cache['ANDROID_PLATFORM'].removeprefix('android-'))
 if revision!='30.0.16248370' or api!=28 or cache['ANDROID_ABI']!='arm64-v8a':raise SystemExit('Android toolchain mismatch')
 manifest.update(android_ndk_revision=revision,android_api=api)
(a.build/'artifact/artifact.json').write_text(json.dumps(manifest,indent=2)+'\n');print(a.build/'artifact/artifact.json')
