//! Foreground match feedback. Uses the existing Android VM, bounded local JNI
//! references and a short one-shot effect. It respects system haptic settings.
use jni::{
    errors::Result,
    objects::{JObject, JValue},
};
use std::ffi::c_void;

#[cfg(target_os = "android")]
pub(super) fn match_found() {
    let Some(app) = bevy::android::ANDROID_APP.get() else {
        return;
    };
    if let Err(error) = vibrate(app.vm_as_ptr(), app.activity_as_ptr()) {
        bevy::log::warn!("Match-found haptic unavailable: {error}");
    }
}

fn vibrate(vm: *mut c_void, activity: *mut c_void) -> Result<()> {
    if vm.is_null() || activity.is_null() {
        return Err(jni::errors::Error::NullPtr("Android activity"));
    }
    // AndroidApp owns these valid VM/global activity references for this call.
    let vm = unsafe { jni::JavaVM::from_raw(vm.cast()) }?;
    let mut env = vm.attach_current_thread()?;
    let activity = unsafe { JObject::from_raw(activity.cast()) };
    let result = env.with_local_frame(32, |env| -> Result<()> {
        let resolver = env
            .call_method(
                &activity,
                "getContentResolver",
                "()Landroid/content/ContentResolver;",
                &[],
            )?
            .l()?;
        let key = env.new_string("haptic_feedback_enabled")?;
        let enabled = env
            .call_static_method(
                "android/provider/Settings$System",
                "getInt",
                "(Landroid/content/ContentResolver;Ljava/lang/String;I)I",
                &[
                    JValue::Object(&resolver),
                    JValue::Object(key.as_ref()),
                    JValue::Int(1),
                ],
            )?
            .i()?;
        if enabled == 0 {
            return Ok(());
        }
        let name = env.new_string("vibrator")?;
        let vibrator = env
            .call_method(
                &activity,
                "getSystemService",
                "(Ljava/lang/String;)Ljava/lang/Object;",
                &[JValue::Object(name.as_ref())],
            )?
            .l()?;
        if vibrator.is_null() || !env.call_method(&vibrator, "hasVibrator", "()Z", &[])?.z()? {
            return Ok(());
        }
        let effect = env
            .call_static_method(
                "android/os/VibrationEffect",
                "createOneShot",
                "(JI)Landroid/os/VibrationEffect;",
                &[JValue::Long(80), JValue::Int(-1)],
            )?
            .l()?;
        env.call_method(
            &vibrator,
            "vibrate",
            "(Landroid/os/VibrationEffect;)V",
            &[JValue::Object(&effect)],
        )?;
        Ok(())
    });
    // A denied/unavailable system service must not poison later JNI calls.
    if env.exception_check()? {
        env.exception_clear()?;
    }
    result
}
