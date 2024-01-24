use std::{
    cell::RefCell,
    io,
    ops::{Deref, DerefMut},
    rc::Rc,
};

use crate::thermal::{ReadWrite, ThermalPortOpener};

use anyhow::anyhow;
use jni::{
    objects::{JClass, JObject},
    sys::jbyte,
};

pub struct AndroidCtx<'a> {
    env: jni::JNIEnv<'a>,
    context: JObject<'a>,
}

impl<'a> AndroidCtx<'a> {
    pub fn new(env: jni::JNIEnv<'a>, context: JObject<'a>) -> Self {
        Self { env, context }
    }
}

pub struct SerialPortOpener<'a> {
    actx: Rc<RefCell<AndroidCtx<'a>>>,
}

pub struct SerialPortReadWrite<'a> {
    actx: Rc<RefCell<AndroidCtx<'a>>>,
    reader: jni::objects::GlobalRef,
}

impl<'a> SerialPortOpener<'a> {
    pub fn new(actx: Rc<RefCell<AndroidCtx<'a>>>) -> Self {
        SerialPortOpener { actx }
    }
}

impl<'a> ThermalPortOpener<'a> for SerialPortOpener<'a> {
    fn open(&mut self) -> anyhow::Result<Box<dyn ReadWrite + 'a>> {
        let mut borrowed_actx = self.actx.borrow_mut();
        let actx = borrowed_actx.deref_mut();

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
            let reader_class: JClass = env
                .call_method(
                    &class_loader,
                    "findClass",
                    "(Ljava/lang/String;)Ljava/lang/Class;",
                    &[class_name.deref().into()],
                )?
                .l()?
                .into();

            let reader = env
                .call_static_method(
                    &reader_class,
                    "open",
                    "()Lcom/github/dobo90/tiop01_gui_android/SerialPortReadWrite;",
                    &[],
                )?
                .l()?;

            if !reader.is_null() {
                Ok(Box::new(SerialPortReadWrite::new(
                    Rc::clone(&self.actx),
                    env.new_global_ref(reader)?,
                )))
            } else {
                Err(anyhow!("open has returned null"))
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
    fn new(actx: Rc<RefCell<AndroidCtx<'a>>>, reader: jni::objects::GlobalRef) -> Self {
        Self { actx, reader }
    }

    fn read(&mut self, buf: &mut [u8]) -> anyhow::Result<usize> {
        let mut borrowed_actx = self.actx.borrow_mut();
        let actx = borrowed_actx.deref_mut();

        let ret = actx.env.with_local_frame(4, |env| {
            let byte_array = env.new_byte_array(buf.len() as i32)?;

            let bytes_read = env
                .call_method(&self.reader, "read", "([B)I", &[byte_array.deref().into()])?
                .i()?;

            if bytes_read > 0 {
                // SAFETY: get_byte_array_region expects &mut [jbyte] that's why
                // we have to cast from &mut [u8] to &mut [i8]
                let buf_slice = unsafe {
                    std::slice::from_raw_parts_mut(buf.as_mut_ptr() as *mut jbyte, buf.len())
                };

                env.get_byte_array_region(byte_array, 0, buf_slice)?;

                Ok(bytes_read as usize)
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
        let mut borrowed_actx = self.actx.borrow_mut();
        let actx = borrowed_actx.deref_mut();

        let ret = actx.env.with_local_frame(4, |env| {
            let byte_array = env.byte_array_from_slice(buf)?;

            let bytes_written = env
                .call_method(&self.reader, "write", "([B)I", &[byte_array.deref().into()])?
                .i()?;

            if bytes_written > 0 {
                Ok(bytes_written as usize)
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
