//! JNI controls share exactly the FRB code-asset library and global Host.
use crate::host::{self, Result, failure};
use jni::{
    JNIEnv,
    objects::{JClass, JIntArray, JLongArray, JObjectArray, JString},
    sys::{jboolean, jlong, jstring},
};
use serde_json::Value;
fn string(env: &mut JNIEnv<'_>, s: JString<'_>) -> Result<String> {
    let value: String = env
        .get_string(&s)
        .map_err(|_| failure("invalid_argument"))?
        .into();
    if value.len() > 16384 {
        return Err(failure("invalid_argument"));
    }
    Ok(value)
}
fn boundary(mut env: JNIEnv<'_>, call: impl FnOnce(&mut JNIEnv<'_>) -> Result<Value>) -> jstring {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| call(&mut env)))
        .unwrap_or_else(|_| Err(failure("cleanup_unconfirmed")));
    env.new_string(host::reply(result))
        .map(|s| s.into_raw())
        .unwrap_or(std::ptr::null_mut())
}
#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_naza3_nexa_verifier_NativeVerifier_nativeBootstrap(
    env: JNIEnv<'_>,
    _: JClass<'_>,
    root: JString<'_>,
    device: JString<'_>,
) -> jstring {
    boundary(env, |env| {
        host::bootstrap(&string(env, root)?, &string(env, device)?)
    })
}
#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_naza3_nexa_verifier_NativeVerifier_nativeVisibility(
    env: JNIEnv<'_>,
    _: JClass<'_>,
    epoch: JString<'_>,
    visible: jboolean,
    sequence: jlong,
) -> jstring {
    boundary(env, |env| {
        host::visibility(&string(env, epoch)?, visible != 0, sequence)
    })
}
#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_naza3_nexa_verifier_NativeVerifier_nativeCancelSelection(
    env: JNIEnv<'_>,
    _: JClass<'_>,
    epoch: JString<'_>,
    token: JString<'_>,
) -> jstring {
    boundary(env, |env| {
        host::cancel_selection(&string(env, epoch)?, &string(env, token)?)
    })
}
#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_naza3_nexa_verifier_NativeVerifier_nativeOpenReport(
    env: JNIEnv<'_>,
    _: JClass<'_>,
    epoch: JString<'_>,
    token: JString<'_>,
) -> jstring {
    boundary(env, |env| {
        host::open_report(&string(env, epoch)?, &string(env, token)?)
    })
}
#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_naza3_nexa_verifier_NativeVerifier_nativeRegisterCandidate(
    env: JNIEnv<'_>,
    _: JClass<'_>,
    epoch: JString<'_>,
    names: JObjectArray<'_>,
    lengths: JLongArray<'_>,
    fds: JIntArray<'_>,
) -> jstring {
    boundary(env, |env| {
        for len in [
            env.get_array_length(&names),
            env.get_array_length(&lengths),
            env.get_array_length(&fds),
        ] {
            if len.map_err(|_| failure("invalid_argument"))? != 5 {
                return Err(failure("invalid_argument"));
            }
        }
        let mut n = Vec::with_capacity(5);
        for i in 0..5 {
            let object = env
                .get_object_array_element(&names, i)
                .map_err(|_| failure("invalid_argument"))?;
            n.push(string(env, JString::from(object))?);
        }
        let mut l = [0i64; 5];
        let mut f = [0i32; 5];
        env.get_long_array_region(&lengths, 0, &mut l)
            .map_err(|_| failure("invalid_argument"))?;
        env.get_int_array_region(&fds, 0, &mut f)
            .map_err(|_| failure("invalid_argument"))?;
        host::register(&string(env, epoch)?, n, l.to_vec(), f.to_vec())
    })
}
