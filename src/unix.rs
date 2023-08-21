use crate::thermal::ThermalPortOpener;

use anyhow::anyhow;
use std::{io, marker::PhantomData, time::Duration};

pub struct SerialPortOpener<'a> {
    phantom: PhantomData<&'a ()>,
}

impl<'a> SerialPortOpener<'a> {
    pub fn new() -> Self {
        Self {
            phantom: PhantomData,
        }
    }
}

impl<'a> ThermalPortOpener<'a> for SerialPortOpener<'a> {
    fn open(&mut self) -> anyhow::Result<Box<dyn io::Read + 'a>> {
        let mut port_path: Option<String> = None;

        for port in serialport::available_ports()? {
            if let serialport::SerialPortType::UsbPort(port_info) = port.port_type {
                if port_info.vid == 0x303a && port_info.pid == 0x4001 {
                    port_path = Some(port.port_name);
                }
            }
        }

        match port_path {
            Some(port_path) => {
                let port = serialport::new(port_path, 921_600)
                    .timeout(Duration::from_secs(1))
                    .open();

                match port {
                    Ok(port) => Ok(Box::new(port)),
                    Err(e) => Err(anyhow!("Failed to open port: {e}")),
                }
            }
            None => Err(anyhow!("Failed to find serial port")),
        }
    }
}
