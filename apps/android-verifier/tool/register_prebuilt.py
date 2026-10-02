"""Hash the real prebuilt asset and all transitive repo inputs for the hook."""
import hashlib,json,os
from pathlib import Path
app=Path(__file__).resolve().parents[1];repo=app.parents[1]
lib=Path(os.environ['CARGO_TARGET_DIR'])/'aarch64-linux-android/release/libnexa_device_verifier.so'
assert lib.is_file()
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
inputs={"pubspec.yaml":sha(app/"pubspec.yaml")}
for root in [app/'rust',repo/'mobile/runtime/crates',repo/'crates/runtime-core',repo/'crates/runtime-types']:
    for p in root.rglob('*'):
        if p.is_file() and (p.suffix=='.rs' or p.name in ('Cargo.toml','Cargo.lock','rust-toolchain.toml')) and 'target' not in p.parts:
            inputs[os.path.relpath(p,app)]=sha(p)
for name in ['native/mnn-patches/lock.json','native/mnn-shim/include/nexa_mnn.h','scripts/android_mnn/candidate-model.json']:
    p=repo/name
    if p.exists():inputs[os.path.relpath(p,app)]=sha(p)
artifact=Path(os.environ['NEXA_MNN_ARTIFACT_DIR'])/'artifact.json'
value={'schema_version':1,'target':'aarch64-linux-android','mode':'release','library_name':lib.name,'path':str(lib.resolve()),'sha256':sha(lib),'native_artifact_sha256':sha(artifact),'inputs':dict(sorted(inputs.items()))}
(app/'build-input').mkdir(exist_ok=True)
(app/'build-input/native.json').write_text(json.dumps(value,indent=2)+'\n')
print(json.dumps({'library_sha256':value['sha256'],'tracked_inputs':len(inputs)}))
