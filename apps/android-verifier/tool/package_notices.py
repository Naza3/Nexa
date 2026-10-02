"""Collect unchanged upstream texts from pinned local build inputs. No downloads.
The Android Maven closure needs its own final inventory/review. A SPDX list is
not substituted for missing original license text. The output is generated.
"""
import hashlib,json,os,re,sys
from pathlib import Path
from urllib.parse import urlparse,unquote
app=Path(__file__).resolve().parents[1];repo=app.parents[1]
metadata=json.load(open(sys.argv[1]));packages={p['id']:p for p in metadata['packages']};nodes={n['id']:n for n in metadata['resolve']['nodes']}
texts=[];inventory=[];missing=[]
def add(component,provenance,path):
    raw=Path(path).read_bytes();text=raw.decode('utf-8',errors='strict')
    inventory.append({'component':component,'source':provenance,'sha256':hashlib.sha256(raw).hexdigest(),'size_bytes':len(raw)})
    texts.append(f'\n\n===== {component} | {provenance} =====\n\n'+text)
manifest=json.load(open(repo/'native/mnn-shim/notices/manifest.json'))
for component in manifest['components']:
    for name in component.get('license_files',[])+component.get('attribution_files',[]):add(component['name'],'native/mnn-shim/notices/'+name,repo/'native/mnn-shim/notices'/name)
add('Nexa MNN modifications','native/mnn-patches/MODIFICATION_NOTICES.md',repo/'native/mnn-patches/MODIFICATION_NOTICES.md')
seen=set()
def visit(i):
    if i in seen:return
    p=packages[i]
    if any('proc-macro' in t['kind'] for t in p['targets']):return
    seen.add(i)
    for dep in nodes[i]['deps']:
        if any(kind['kind'] is None for kind in dep['dep_kinds']):visit(dep['pkg'])
visit(metadata['resolve']['root'])
for i in sorted(seen):
    p=packages[i]
    if not p['source']:continue
    root=Path(p['manifest_path']).parent;found=[]
    for f in root.iterdir():
        if f.is_file() and re.match(r'^(licen[cs]e|copying|copyright|notice)([._-]|$)',f.name,re.I):found.append(f)
    if p.get('license_file'):
        f=root/p['license_file']
        if f not in found:found.append(f)
    if p['name']=='flutter_rust_bridge':found=[Path(os.environ['PUB_CACHE'])/'hosted/pub.dev/flutter_rust_bridge-2.13.0/LICENSE']
    if p['name']=='dart-sys':
        found += [root/'dart-sdk/LICENSE']
        wrapper=list((app/'assets/upstream').glob('dart-sys-LICENSE*'))
        found += wrapper
        if not wrapper:missing.append({'component':'dart-sys wrapper 4.1.5','declared':'MIT OR Apache-2.0','reason':'exact repository license originals required in addition to Dart SDK license'})
    if not found:missing.append({'component':p['name']+' '+p['version'],'declared':p['license']})
    for f in sorted(found):
        if f.is_relative_to(root):provenance='crates.io/'+p['name']+'/'+p['version']+'/'+f.relative_to(root).as_posix()
        elif f.parent==app/'assets/upstream':provenance=json.loads((app/'assets/upstream/SOURCES.json').read_text())['sources'][f.name]
        else:provenance='pub.dev/flutter_rust_bridge/2.13.0/LICENSE (same upstream workspace MIT text)'
        add(p['name']+' '+p['version'],provenance,f)
config=json.load(open(app/'.dart_tool/package_config.json'))
for p in config['packages']:
    root=Path(unquote(urlparse(p['rootUri']).path))
    if not root.is_absolute():root=(app/'.dart_tool'/root).resolve()
    if p['name'] in ['android_verifier','flutter_test','flutter_lints','test_api','matcher','fake_async','leak_tracker','leak_tracker_flutter_testing','leak_tracker_testing','vm_service']:continue
    for f in sorted(root.glob('LICENSE*')):
        if f.is_file():add('Dart package '+p['name']+' ('+root.name+')','pub/'+root.name+'/'+f.name,f)
flutter=Path(os.environ['FLUTTER_ROOT'])
add('Flutter 3.47.6 engine and third-party code','Flutter SDK sky_engine/LICENSE',flutter/'bin/cache/pkg/sky_engine/LICENSE')
add('Dart 3.13.5 SDK','Dart SDK LICENSE',flutter/'bin/cache/dart-sdk/LICENSE')
for f in (flutter/'bin/cache/artifacts/material_fonts').glob('*LICENSE*'):add('Flutter bundled font','material_fonts/'+f.name,f)
rust=Path(os.environ['RUSTUP_HOME'])/'toolchains/1.98.1-x86_64-unknown-linux-gnu/share/doc/rust/licenses'
for name in ['MIT.txt','Apache-2.0.txt','Unicode-3.0.txt','LLVM-exception.txt']:
    if (rust/name).exists():add('Rust 1.98.1 standard library','Rust distribution licenses/'+name,rust/name)
for f in sorted((app/'assets/upstream').glob('kotlin-*')):add('Kotlin 2.4.0','JetBrains/kotlin/v2.4.0/license/'+f.name,f)
if not (assets_path:=app/'assets/maven-notices.txt').is_file():raise SystemExit('Actual Maven runtime notice closure is required')
add('Android Maven runtime closure','assets/maven-notices.txt',assets_path)
header='Nexa 设备验证 / Android device verification research package\nresearch_only=true; production_admitted=false\n\nUnchanged upstream license and attribution texts follow. No model is included in this APK. The imported candidate remains governed by its publisher license. Flutter also bundles its generated NOTICES.Z.\n'
(app/'assets').mkdir(exist_ok=True)
(app/'assets/THIRD_PARTY_NOTICES.txt').write_text(header+''.join(texts))
(app/'assets/notice-inventory.json').write_text(json.dumps({'schema_version':1,'scope':'Native/Rust/Dart and actual Maven releaseRuntimeClasspath notice texts; final APK bytes require independent audit','files':inventory,'missing':missing},indent=2)+'\n')
print(json.dumps({'texts':len(inventory),'missing':missing,'bytes':(app/'assets/THIRD_PARTY_NOTICES.txt').stat().st_size}))
if missing:sys.exit(1)
