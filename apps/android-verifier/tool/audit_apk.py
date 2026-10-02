"""Verify the actual APK, every bundled ELF, the code asset, and notice bytes."""
import hashlib,json,os,re,struct,subprocess,sys,zipfile
from pathlib import Path
app=Path(__file__).resolve().parents[1];apk=Path(sys.argv[1]).resolve();out=Path(sys.argv[2]).resolve();out.mkdir(parents=True,exist_ok=True)
ndk=Path(os.environ['ANDROID_NDK_HOME'])/'toolchains/llvm/prebuilt/linux-x86_64/bin';sdk=Path(os.environ['ANDROID_HOME'])/'build-tools/36.0.0'
def run(args):return subprocess.check_output(list(map(str,args)),text=True,stderr=subprocess.STDOUT)
def sha(data):return hashlib.sha256(data).hexdigest()
pre=json.loads((app/'build-input/native.json').read_text());assert sha(Path(pre['path']).read_bytes())==pre['sha256']
for name,digest in pre['inputs'].items():assert sha((app/name).read_bytes())==digest,('stale input',name)
with zipfile.ZipFile(apk) as z,apk.open('rb') as raw:
    names=z.namelist();libs=[n for n in names if n.endswith('.so')]
    assert sorted(libs)==['lib/arm64-v8a/libapp.so','lib/arm64-v8a/libflutter.so','lib/arm64-v8a/libnexa_device_verifier.so']
    assert not any(n.endswith(('.mnn','.mnn.weight','.gguf')) for n in names)
    dex=b''.join(z.read(name) for name in names if re.fullmatch(r'classes(?:\d+)?\.dex',name))
    assert b'nexa-device-report.txt' in dex and b'text/plain' in dex
    assert b'nexa-device-report.json' not in dex
    elfs=[]
    for name in libs:
        entry=z.getinfo(name);assert entry.compress_type==zipfile.ZIP_STORED
        raw.seek(entry.header_offset);header=raw.read(30);n,e=struct.unpack_from('<HH',header,26);offset=entry.header_offset+30+n+e;assert offset%16384==0
        file=out/Path(name).name;data=z.read(name);file.write_bytes(data)
        hdr=run([ndk/'llvm-readelf','-hW',file]);assert 'AArch64' in hdr
        segments=run([ndk/'llvm-readelf','-lW',file]);loads=[];relros=[]
        for line in segments.splitlines():
            fields=line.split()
            if not fields:continue
            if fields[0]=='LOAD':
                values={'offset':int(fields[1],16),'vaddr':int(fields[2],16),'memsz':int(fields[5],16),'align':int(fields[-1],16)}
                assert values['align']>=16384 and (values['vaddr']-values['offset'])%16384==0;loads.append(values)
            if fields[0]=='GNU_RELRO':
                start=int(fields[2],16);end=start+int(fields[5],16);relros.append({'start':start,'end':end,'loader_16k_start':start//16384*16384,'loader_16k_end':(end+16383)//16384*16384})
        assert loads
        if name.endswith('libapp.so'):
            assert not relros
            relocations=run([ndk/'llvm-readelf','-rW',file]);assert 'There are no relocations' in relocations
            sections=run([ndk/'llvm-readelf','-SW',file]);assert not re.search(r'\.(?:got|plt)(?:\.|\s)',sections)
            relro_status='not_present_dart_aot_no_import_relocations_got_or_plt'
        else:
            assert relros
            relro_status='present_loader_rounds_to_device_page'
        if name.endswith('libnexa_device_verifier.so'):
            assert all(r['end']%16384==0 for r in relros)
            relro_status='present_strict_16k_endpoint' 
        needed=re.findall(r'Shared library: \[(.*?)\]',run([ndk/'llvm-readelf','-dW',file]));assert 'libc++_shared.so' not in needed
        if name.endswith('libnexa_device_verifier.so'):
            assert set(needed)=={'libm.so','libdl.so','libc.so'}
            symbols=run([ndk/'llvm-nm','-D',file])
            for method in ['nativeBootstrap','nativeRegisterCandidate','nativeCancelSelection','nativeVisibility','nativeOpenReport']:assert 'Java_io_github_naza3_nexa_verifier_NativeVerifier_'+method in symbols
            assert 'frb_' in symbols
            stripped=out/'registered-stripped.so';subprocess.run([str(ndk/'llvm-strip'),'--strip-unneeded','-o',str(stripped),pre['path']],check=True);assert stripped.read_bytes()==data
        elfs.append({'path':name,'sha256':sha(data),'bytes':len(data),'zip_data_offset':offset,'loads':loads,'relro':relros,'relro_status':relro_status,'needed':needed})
    notice=z.read('assets/flutter_assets/assets/THIRD_PARTY_NOTICES.txt');assert notice==(app/'assets/THIRD_PARTY_NOTICES.txt').read_bytes();assert 'assets/flutter_assets/NOTICES.Z' in names
badging=run([sdk/'aapt2','dump','badging',apk]);(out/'badging.txt').write_text(badging)
assert "name='io.github.naza3.nexa.verifier'" in badging and "minSdkVersion:'28'" in badging and "targetSdkVersion:'36'" in badging
version=re.search(r'^version: (\d+\.\d+\.\d+)\+(\d+)$',(app/'pubspec.yaml').read_text(),re.M);assert version
assert "versionName='"+version[1]+"'" in badging and "versionCode='"+version[2]+"'" in badging
permissions=re.findall(r"uses-permission: name='([^']+)'",badging)
assert permissions==['io.github.naza3.nexa.verifier.DYNAMIC_RECEIVER_NOT_EXPORTED_PERMISSION']
manifest=run([sdk/'aapt2','dump','xmltree',apk,'--file','AndroidManifest.xml']);(out/'manifest.txt').write_text(manifest)
for attribute in ['allowBackup','fullBackupContent','usesCleartextTraffic','extractNativeLibs']:
    assert re.search(r'android:'+attribute+r'[^\n]*=false',manifest),attribute
for key in ['nexa.research_only','nexa.production_admitted','android_device_verification']:assert key in manifest
for key,value in [('nexa.research_only','true'),('nexa.production_admitted','false')]:
    blocks=[b for b in manifest.split('E: meta-data') if '"'+key+'"' in b]
    assert len(blocks)==1 and re.search(r'android:value[^\n]*='+value,blocks[0])
assert 'android:debuggable' not in manifest or re.search(r'android:debuggable[^\n]*0x0',manifest)
signature=run([sdk/'apksigner','verify','--verbose','--print-certs',apk]);(out/'signature.txt').write_text(signature)
assert 'Verified using v2 scheme (APK Signature Scheme v2): true' in signature and 'CN=Android Debug' in signature
alignment=run([sdk/'zipalign','-c','-P','16','-v','4',apk]);(out/'zipalign.txt').write_text(alignment);assert 'Verification successful' in alignment
result={'schema_version':1,'apk_sha256':sha(apk.read_bytes()),'apk_bytes':apk.stat().st_size,'application_id':'io.github.naza3.nexa.verifier','version':version[0].removeprefix('version: '),'build_mode':'release','signing_kind':'internal_debug_key','signer_sha256':re.search(r'certificate SHA-256 digest: (\w+)',signature).group(1),'purpose':'android_device_verification','research_only':True,'production_admitted':False,'device_execution':'not_run','report_export':{'filename':'nexa-device-report.txt','mime':'text/plain','payload':'unchanged_json'},'manifest_permissions':permissions,'prebuilt_sha256':pre['sha256'],'native_artifact_sha256':pre['native_artifact_sha256'],'notice_sha256':sha(notice),'source_inputs':pre['inputs'],'elfs':elfs}
(out/'audit.json').write_text(json.dumps(result,indent=2)+'\n');print(json.dumps({k:v for k,v in result.items() if k not in ['source_inputs','elfs']}))
