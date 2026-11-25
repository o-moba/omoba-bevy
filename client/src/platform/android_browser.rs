use jni::{
    errors::{Error, Result},
    objects::{JObject, JValue},
};

pub(super) fn open(url: &str) -> Result<()> {
    let app = bevy::android::ANDROID_APP
        .get()
        .ok_or(Error::NullPtr("Android activity"))?;
    // AndroidApp owns the process VM and the global Activity reference.
    let vm = unsafe { jni::JavaVM::from_raw(app.vm_as_ptr().cast()) }?;
    let mut env = vm.attach_current_thread()?;
    let activity = unsafe { JObject::from_raw(app.activity_as_ptr().cast()) };
    let result = env.with_local_frame(8, |env| -> Result<()> {
        let url = env.new_string(url)?;
        let uri = env
            .call_static_method(
                "android/net/Uri",
                "parse",
                "(Ljava/lang/String;)Landroid/net/Uri;",
                &[JValue::Object(url.as_ref())],
            )?
            .l()?;
        let action = env.new_string("android.intent.action.VIEW")?;
        let intent = env.new_object(
            "android/content/Intent",
            "(Ljava/lang/String;Landroid/net/Uri;)V",
            &[JValue::Object(action.as_ref()), JValue::Object(&uri)],
        )?;
        env.call_method(
            &activity,
            "startActivity",
            "(Landroid/content/Intent;)V",
            &[JValue::Object(&intent)],
        )?;
        Ok(())
    });
    if env.exception_check()? {
        env.exception_clear()?;
    }
    result
}
