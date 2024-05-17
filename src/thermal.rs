use crate::app::{ConnectionStatus, ProducerMessage, UiMessage};
use crate::image_utils::{self, map_to_scaled_value};
use crate::thermal;

use byteorder::{LittleEndian, ReadBytesExt};
use eframe::egui;
use image2::Kernel;
use scarlet::colormap::{GradientColorMap, ListedColorMap};
use std::io::Write;
use std::sync::mpsc::{Receiver, Sender, TryRecvError};
use std::time::Duration;
use std::{io, thread};
use strum_macros::{Display, EnumIter};

pub type GrayImage = image2::Image<u16, image2::Gray>;
pub type RgbImage = image2::Image<u8, image2::Rgb>;

pub const THERMAL_IMAGE_WIDTH: usize = 32;
pub const THERMAL_IMAGE_HEIGHT: usize = 32;
pub const THERMAL_IMAGE_SIZE: [usize; 2] = [THERMAL_IMAGE_WIDTH, THERMAL_IMAGE_HEIGHT];

#[derive(Debug, Display, Clone, PartialEq, EnumIter)]
pub enum FilteringMethod {
    None,
    #[strum(to_string = "Box 3x3")]
    Box3x3,
    #[strum(to_string = "Gaussian 3x3")]
    Gaussian3x3,
}

#[derive(Debug, Display, Clone, PartialEq, EnumIter)]
pub enum EdgeStrategy {
    Constant,
    Extend,
    Wrap,
    Mirror,
}

#[derive(Debug, Display, Clone, PartialEq, EnumIter)]
pub enum ColorMap {
    Turbo,
    Magma,
    #[strum(to_string = "Blue Red")]
    Bluered,
    Breeze,
    Mist,
    #[strum(to_string = "Blue Red (linear)")]
    LinearBlueRed,
    #[strum(to_string = "Black White (linear)")]
    LinearBlackWhite,
}

impl FilteringMethod {
    fn get_kernel(&self) -> Option<image2::Kernel> {
        match self {
            FilteringMethod::None => None,
            FilteringMethod::Box3x3 => {
                let mut kernel = image2::Kernel::create(3, 3, |_x, _y| 1.);
                kernel.normalize();
                Some(kernel)
            }
            FilteringMethod::Gaussian3x3 => Some(image2::Kernel::gaussian_3x3()),
        }
    }
}

impl EdgeStrategy {
    fn get_edge_strategy(&self) -> image2::kernel::EdgeStrategy {
        match self {
            EdgeStrategy::Constant => image2::kernel::EdgeStrategy::Constant,
            EdgeStrategy::Extend => image2::kernel::EdgeStrategy::Extend,
            EdgeStrategy::Wrap => image2::kernel::EdgeStrategy::Wrap,
            EdgeStrategy::Mirror => image2::kernel::EdgeStrategy::Mirror,
        }
    }
}

impl ColorMap {
    pub fn get_colormap(
        &self,
    ) -> Box<dyn scarlet::colormap::ColorMap<scarlet::color::RGBColor> + Sync> {
        match self {
            ColorMap::Turbo => Box::new(ListedColorMap::turbo()),
            ColorMap::Magma => Box::new(ListedColorMap::magma()),
            ColorMap::Bluered => Box::new(ListedColorMap::bluered()),
            ColorMap::Breeze => Box::new(ListedColorMap::breeze()),
            ColorMap::Mist => Box::new(ListedColorMap::mist()),
            ColorMap::LinearBlueRed => {
                let blue = scarlet::color::RGBColor::from_hex_code("#0000FF").unwrap();
                let red = scarlet::color::RGBColor::from_hex_code("#FF0000").unwrap();
                Box::new(GradientColorMap::new_linear(blue, red))
            }
            ColorMap::LinearBlackWhite => {
                let black = scarlet::color::RGBColor::from_hex_code("#000000").unwrap();
                let white = scarlet::color::RGBColor::from_hex_code("#FFFFFF").unwrap();
                Box::new(GradientColorMap::new_linear(black, white))
            }
        }
    }
}

#[derive(Debug, PartialEq, Clone)]
pub struct Settings {
    pub flip_horizontally: bool,
    pub flip_vertically: bool,
    pub filtering_method: FilteringMethod,
    pub edge_strategy: EdgeStrategy,
    pub colormap: ColorMap,
    pub emissivity: u8,
    pub color_range: u8,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            flip_horizontally: false,
            flip_vertically: false,
            filtering_method: FilteringMethod::Box3x3,
            edge_strategy: EdgeStrategy::Extend,
            colormap: ColorMap::Turbo,
            emissivity: 95,
            color_range: 100,
        }
    }
}

impl Settings {
    fn get_kernel(&self) -> Option<image2::Kernel> {
        let mut kernel = self.filtering_method.get_kernel();

        if let Some(ref mut kernel) = kernel {
            kernel.set_edge_strategy(self.edge_strategy.get_edge_strategy());
        }

        kernel
    }
}

pub trait PortOpener<'a> {
    type RW: io::Read + io::Write + 'a;

    fn open(&mut self) -> anyhow::Result<Self::RW>;
}

pub struct Frame {
    pub image: thermal::RgbImage,
    pub min: f64,
    pub max: f64,
}

pub struct ImageProducer<'a, T>
where
    T: PortOpener<'a>,
{
    opener: T,
    rw: Option<T::RW>,
    settings: Settings,
    kernel: Option<Kernel>,
    colormap: Box<dyn scarlet::colormap::ColorMap<scarlet::color::RGBColor> + Sync>,
    sender: Sender<ProducerMessage>,
    receiver: Receiver<UiMessage>,
    egui_ctx: egui::Context,
}

impl<'a, T> ImageProducer<'a, T>
where
    T: PortOpener<'a>,
{
    pub fn new(
        egui_ctx: egui::Context,
        sender: Sender<ProducerMessage>,
        receiver: Receiver<UiMessage>,
        opener: T,
    ) -> Self {
        let settings = Settings::default();
        let kernel = settings.get_kernel();
        let colormap = settings.colormap.get_colormap();
        let rw = None;

        Self {
            opener,
            rw,
            settings,
            kernel,
            colormap,
            sender,
            receiver,
            egui_ctx,
        }
    }

    #[profiling::function]
    fn ensure_port_opened(&mut self) {
        if self.rw.is_some() {
            return;
        }

        match self.opener.open() {
            Ok(rw) => {
                self.rw = Some(rw);
                self.write_emissivity();
                self.send_message_to_ui(ProducerMessage::ConnectionStatusChange(
                    ConnectionStatus::Connected,
                ));
            }
            Err(e) => {
                log::warn!("Failed to create rw: {e}. Sleeping for 1 sec");
                thread::sleep(Duration::from_secs(1));
            }
        }
    }

    #[profiling::function]
    fn read_image(&mut self) -> Option<thermal::GrayImage> {
        let mut imgbuf = thermal::GrayImage::new(THERMAL_IMAGE_SIZE);

        let r = self
            .rw
            .as_mut()?
            .read_u16_into::<LittleEndian>(imgbuf.data_mut());

        match r {
            Ok(()) => Some(imgbuf),
            Err(e) => {
                log::error!("Failed to read from serial port: {e}");

                self.rw = None;
                self.send_message_to_ui(ProducerMessage::ConnectionStatusChange(
                    ConnectionStatus::Disconnected,
                ));

                None
            }
        }
    }

    #[profiling::function]
    fn produce_thermal_frame(&self, gray_image: &thermal::GrayImage) {
        let filtered = {
            profiling::scope!("filter");
            self.kernel
                .as_ref()
                .map(|kernel| gray_image.run(kernel.clone(), None))
        };

        let filtered = filtered.as_ref().unwrap_or(gray_image);
        let color_range = self.settings.color_range;

        if let Some((min, max)) = {
            profiling::scope!("minmax");
            let min = filtered.iter().map(|(_pt, data)| data.as_slice()[0]).min();
            let max = filtered.iter().map(|(_pt, data)| data.as_slice()[0]).max();
            min.zip(max)
        } {
            let mut imgbuf = thermal::RgbImage::new(THERMAL_IMAGE_SIZE);

            {
                profiling::scope!("colorize");
                imgbuf.each_pixel_mut(|pt, pixel| {
                    let current_pixel = filtered.get([pt.x, pt.y]).as_slice()[0];
                    let scaled_value = map_to_scaled_value(current_pixel, min, max, color_range);

                    let color = self.colormap.transform_single(scaled_value);
                    pixel.copy_from_slice([color.int_r(), color.int_g(), color.int_b()]);
                });
            }

            if self.settings.flip_horizontally {
                profiling::scope!("horizontal flip");
                imgbuf.run_in_place(image_utils::Flip::Horizontal);
            }
            if self.settings.flip_vertically {
                profiling::scope!("vertical flip");
                imgbuf.run_in_place(image_utils::Flip::Vertical);
            }

            self.send_message_to_ui(ProducerMessage::Frame(Frame {
                image: imgbuf,
                min: f64::from(min) / 10.0,
                max: f64::from(max) / 10.0,
            }));
        }
    }

    #[profiling::function]
    fn write_emissivity(&mut self) {
        if let Some(ref mut rw) = self.rw {
            let command: [u8; 4] = [
                0x55,
                0x01,
                self.settings.emissivity,
                0x56 + self.settings.emissivity,
            ];

            let _ = rw
                .write_all(&command)
                .inspect_err(|e| log::error!("Failed to write emissivity {e}"));
        }
    }

    #[profiling::function]
    fn send_message_to_ui(&self, message: ProducerMessage) {
        if self.sender.send(message).is_ok() {
            self.egui_ctx.request_repaint();
        }
    }

    pub fn main_loop(&mut self) {
        loop {
            self.ensure_port_opened();

            let new_settings: Option<Settings> = {
                profiling::scope!("receive settings");
                let mut received_settings: Option<Settings> = None;

                loop {
                    match self.receiver.try_recv() {
                        Ok(UiMessage::ChangeSettings(settings)) => {
                            received_settings = Some(settings);
                        }
                        Err(TryRecvError::Disconnected | TryRecvError::Empty) => {
                            break received_settings
                        }
                    }
                }
            };

            if let Some(ref new_settings) = new_settings {
                profiling::scope!("apply settings");
                self.settings = new_settings.clone();
                self.kernel = self.settings.get_kernel();
                self.colormap = self.settings.colormap.get_colormap();
                self.write_emissivity();
            }

            if let Some(ref gray_image) = self.read_image() {
                self.produce_thermal_frame(gray_image);
            }

            profiling::finish_frame!();
        }
    }
}
