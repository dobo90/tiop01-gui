use crate::thermal::{ReadWrite, ThermalPortOpener};

use anyhow::anyhow;
use serialport::SerialPort;
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

struct ThermalReadWrite(Box<dyn SerialPort>);

impl io::Read for ThermalReadWrite {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, io::Error> {
        self.0.read(buf)
    }
}

impl io::Write for ThermalReadWrite {
    fn write(&mut self, buf: &[u8]) -> Result<usize, io::Error> {
        self.0.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }
}

impl ReadWrite for ThermalReadWrite {}

impl<'a> ThermalPortOpener<'a> for SerialPortOpener<'a> {
    fn open(&mut self) -> anyhow::Result<Box<dyn ReadWrite + 'a>> {
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
                    Ok(port) => Ok(Box::new(ThermalReadWrite(port))),
                    Err(e) => Err(anyhow!("Failed to open port: {e}")),
                }
            }
            None => Err(anyhow!("Failed to find serial port")),
        }
    }
}
