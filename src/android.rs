use std::{
    cell::RefCell,
    io,
    ops::{Deref, DerefMut},
    rc::Rc,
};

use crate::thermal::ThermalPortOpener;

use anyhow::anyhow;
use jni::objects::{JClass, JObject};

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

pub struct SerialPortReader<'a> {
    actx: Rc<RefCell<AndroidCtx<'a>>>,
    reader: jni::objects::GlobalRef,
}

impl<'a> SerialPortOpener<'a> {
    pub fn new(actx: Rc<RefCell<AndroidCtx<'a>>>) -> Self {
        SerialPortOpener { actx }
    }
}

impl<'a> ThermalPortOpener<'a> for SerialPortOpener<'a> {
    fn open(&mut self) -> anyhow::Result<Box<dyn io::Read + 'a>> {
        let mut borrow_actx = self.actx.borrow_mut();
        let actx = borrow_actx.deref_mut();

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
                env.new_string("com/github/dobo90/tiop01_gui_android/SerialPortReader")?;
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
                    "openReader",
                    "()Lcom/github/dobo90/tiop01_gui_android/SerialPortReader;",
                    &[],
                )?
                .l()?;

            if !reader.is_null() {
                Ok(Box::new(SerialPortReader::new(
                    Rc::clone(&self.actx),
                    env.new_global_ref(reader)?,
                )))
            } else {
                Err(anyhow!("openReader has returned null"))
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

impl<'a> SerialPortReader<'a> {
    fn new(actx: Rc<RefCell<AndroidCtx<'a>>>, reader: jni::objects::GlobalRef) -> Self {
        Self { actx, reader }
    }

    fn read(&mut self, buf: &mut [u8]) -> anyhow::Result<usize> {
        let mut borrow_actx = self.actx.borrow_mut();
        let actx = borrow_actx.deref_mut();

        let ret = actx.env.with_local_frame(4, |env| {
            let byte_array = env.new_byte_array(buf.len() as i32).unwrap();

            let bytes_read = env
                .call_method(&self.reader, "read", "([B)I", &[byte_array.deref().into()])?
                .i()?;

            if bytes_read > 0 {
                // TODO: copy directly to buf (to avoid one additional copy)
                let vec = env.convert_byte_array(byte_array).unwrap();
                buf.copy_from_slice(&vec);

                Ok(bytes_read as usize)
            } else {
                Err(anyhow!("Reader has returned negative value: {bytes_read}"))
            }
        });

        match ret {
            Ok(ret) => Ok(ret),
            Err(e) => Err(anyhow!("{e}")),
        }
    }
}

impl<'a> io::Read for SerialPortReader<'a> {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, io::Error> {
        match self.read(buf) {
            Ok(r) => Ok(r),
            Err(e) => {
                log::error!("SerialPortReader::read failed: {e}");
                Err(io::ErrorKind::Other.into())
            }
        }
    }
}
