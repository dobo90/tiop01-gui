use std::{cell::RefCell, io, ops::Deref, rc::Rc};

use crate::thermal::{PortOpener, ReadWrite};

use anyhow::anyhow;
use jni::{
    objects::{JClass, JObject},
    sys::jbyte,
};

pub struct Context<'a> {
    env: jni::JNIEnv<'a>,
    context: JObject<'a>,
}

impl<'a> Context<'a> {
    pub fn new(env: jni::JNIEnv<'a>, context: JObject<'a>) -> Self {
        Self { env, context }
    }
}

pub struct SerialPortOpener<'a> {
    actx: Rc<RefCell<Context<'a>>>,
}

pub struct SerialPortReadWrite<'a> {
    actx: Rc<RefCell<Context<'a>>>,
    rw: jni::objects::GlobalRef,
}

impl<'a> SerialPortOpener<'a> {
    pub fn new(actx: Rc<RefCell<Context<'a>>>) -> Self {
        SerialPortOpener { actx }
    }
}

impl<'a> PortOpener<'a> for SerialPortOpener<'a> {
    fn open(&mut self) -> anyhow::Result<Box<dyn ReadWrite + 'a>> {
        let actx = &mut *self.actx.borrow_mut();

        let ret = actx.env.with_local_frame(4, |env| {
            let class_loader = env
                .call_method(
                    &actx.context,
                    "getClassLoader",
                    "()Ljava/lang/ClassLoader;",
                    &[],
                )?
                .l()?;

            let class_name =
                env.new_string("com/github/dobo90/tiop01_gui_android/SerialPortReadWrite")?;
            let rw_class: JClass = env
                .call_method(
                    &class_loader,
                    "findClass",
                    "(Ljava/lang/String;)Ljava/lang/Class;",
                    &[class_name.deref().into()],
                )?
                .l()?
                .into();

            let rw = env
                .call_static_method(
                    &rw_class,
                    "open",
                    "()Lcom/github/dobo90/tiop01_gui_android/SerialPortReadWrite;",
                    &[],
                )?
                .l()?;

            if rw.is_null() {
                Err(anyhow!("open has returned null"))
            } else {
                Ok(Box::new(SerialPortReadWrite::new(
                    Rc::clone(&self.actx),
                    env.new_global_ref(rw)?,
                )))
            }
        });

        match ret {
            Ok(ret) => Ok(ret),
            Err(e) => {
                log::error!("SerialPortOpener::open failed: {e}");
                Err(e)
            }
        }
    }
}

impl<'a> SerialPortReadWrite<'a> {
    fn new(actx: Rc<RefCell<Context<'a>>>, rw: jni::objects::GlobalRef) -> Self {
        Self { actx, rw }
    }

    fn read(&mut self, buf: &mut [u8]) -> anyhow::Result<usize> {
        let actx = &mut *self.actx.borrow_mut();

        let ret = actx.env.with_local_frame(4, |env| {
            let byte_array = env.new_byte_array(i32::try_from(buf.len())?)?;

            let bytes_read = env
                .call_method(&self.rw, "read", "([B)I", &[byte_array.deref().into()])?
                .i()?;

            if bytes_read > 0 {
                // SAFETY: get_byte_array_region expects &mut [jbyte] that's why
                // we have to cast from &mut [u8] to &mut [i8]
                let buf_slice = unsafe {
                    std::slice::from_raw_parts_mut(buf.as_mut_ptr().cast::<jbyte>(), buf.len())
                };

                env.get_byte_array_region(byte_array, 0, buf_slice)?;

                Ok(usize::try_from(bytes_read)?)
            } else {
                Err(anyhow!("JNI read failed: {bytes_read}"))
            }
        });

        match ret {
            Ok(ret) => Ok(ret),
            Err(e) => Err(anyhow!("{e}")),
        }
    }

    fn write(&mut self, buf: &[u8]) -> anyhow::Result<usize> {
        let actx = &mut *self.actx.borrow_mut();

        let ret = actx.env.with_local_frame(4, |env| {
            let byte_array = env.byte_array_from_slice(buf)?;

            let bytes_written = env
                .call_method(&self.rw, "write", "([B)I", &[byte_array.deref().into()])?
                .i()?;

            if bytes_written > 0 {
                Ok(usize::try_from(bytes_written)?)
            } else {
                Err(anyhow!("JNI write failed: {bytes_written}"))
            }
        });

        match ret {
            Ok(ret) => Ok(ret),
            Err(e) => Err(anyhow!("{e}")),
        }
    }
}

impl<'a> io::Read for SerialPortReadWrite<'a> {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, io::Error> {
        match self.read(buf) {
            Ok(n) => Ok(n),
            Err(e) => {
                log::error!("SerialPortReadWrite::read failed: {e}");
                Err(io::ErrorKind::Other.into())
            }
        }
    }
}

impl<'a> io::Write for SerialPortReadWrite<'a> {
    fn write(&mut self, buf: &[u8]) -> Result<usize, io::Error> {
        match self.write(buf) {
            Ok(n) => Ok(n),
            Err(e) => {
                log::error!("SerialPortReadWrite::write failed: {e}");
                Err(io::ErrorKind::Other.into())
            }
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<'a> ReadWrite for SerialPortReadWrite<'a> {}
