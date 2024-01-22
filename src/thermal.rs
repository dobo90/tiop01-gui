use crate::app::{ProducerMessage, UiMessage};
use crate::image_utils::{self, map_to_scaled_value};
use crate::thermal;

use byteorder::{LittleEndian, ReadBytesExt};
use eframe::egui;
use image2::Kernel;
use itertools::{Itertools, MinMaxResult};
use scarlet::colormap::{GradientColorMap, ListedColorMap};
use std::sync::mpsc::{Receiver, Sender, TryRecvError};
use std::time::Duration;
use std::{io, thread};
use strum_macros::EnumIter;

pub type GrayImage = image2::Image<u16, image2::Gray>;
pub type RgbImage = image2::Image<u8, image2::Rgb>;

pub const THERMAL_IMAGE_WIDTH: usize = 32;
pub const THERMAL_IMAGE_HEIGHT: usize = 32;
pub const THERMAL_IMAGE_SIZE: [usize; 2] = [THERMAL_IMAGE_WIDTH, THERMAL_IMAGE_HEIGHT];

#[derive(Debug, Clone, PartialEq, EnumIter)]
pub enum FilteringMethod {
    None,
    Box3x3,
    Gaussian3x3,
}

#[derive(Debug, Clone, PartialEq, EnumIter)]
pub enum EdgeStrategy {
    Constant,
    Extend,
    Wrap,
    Mirror,
}

#[derive(Debug, Clone, PartialEq, EnumIter)]
pub enum ColorMap {
    Turbo,
    Magma,
    Bluered,
    Breeze,
    Mist,
    LinearBlueRed,
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

        if let Some(kernel) = &mut kernel {
            kernel.set_edge_strategy(self.edge_strategy.get_edge_strategy());
        }

        kernel
    }
}

pub trait ReadWrite: io::Read + io::Write {}

pub trait ThermalPortOpener<'a> {
    fn open(&mut self) -> anyhow::Result<Box<dyn ReadWrite + 'a>>;
}

pub struct Frame {
    pub image: thermal::RgbImage,
    pub min: f64,
    pub max: f64,
}

pub struct ThermalImageProducer<'a, T: ThermalPortOpener<'a> + 'a> {
    opener: T,
    rw: Option<Box<dyn ReadWrite + 'a>>,
    settings: Settings,
    kernel: Option<Kernel>,
    sender: Sender<ProducerMessage>,
    receiver: Receiver<UiMessage>,
    egui_ctx: egui::Context,
}

impl<'a, T: ThermalPortOpener<'a> + 'a> ThermalImageProducer<'a, T> {
    pub fn new(
        egui_ctx: egui::Context,
        sender: Sender<ProducerMessage>,
        receiver: Receiver<UiMessage>,
        opener: T,
    ) -> Self {
        let settings = Settings::default();
        let kernel = settings.get_kernel();
        let rw = None;
        Self {
            opener,
            rw,
            settings,
            kernel,
            sender,
            receiver,
            egui_ctx,
        }
    }

    fn ensure_port_opened(&mut self) {
        if self.rw.is_some() {
            return;
        }

        match self.opener.open() {
            Ok(rw) => {
                self.rw = Some(rw);
                self.write_emissivity();
            }
            Err(e) => {
                log::warn!("Failed to create rw: {e}. Sleeping for 1 sec");
                thread::sleep(Duration::from_secs(1));
            }
        }
    }

    fn read_image(&mut self) -> Option<thermal::GrayImage> {
        let mut imgbuf = thermal::GrayImage::new(THERMAL_IMAGE_SIZE);

        let r = self
            .rw
            .as_mut()
            .unwrap()
            .read_u16_into::<LittleEndian>(imgbuf.data_mut());

        match r {
            Ok(_) => Some(imgbuf),
            Err(e) => {
                log::error!("Failed to read from serial port: {e}");
                self.rw = None;
                None
            }
        }
    }

    fn produce_thermal_frame(&self, gray_image: &thermal::GrayImage) {
        let filtered = self
            .kernel
            .as_ref()
            .map(|kernel| gray_image.run(kernel.clone(), None));

        let filtered = filtered.as_ref().unwrap_or(gray_image);
        let color_range = self.settings.color_range;

        if let MinMaxResult::MinMax(min, max) = filtered
            .iter()
            .map(|(_pt, data)| data.as_slice()[0])
            .minmax()
        {
            let mut imgbuf = thermal::RgbImage::new(THERMAL_IMAGE_SIZE);
            let colormap = self.settings.colormap.get_colormap();

            imgbuf.each_pixel_mut(|pt, pixel| {
                let current_pixel = filtered.get([pt.x, pt.y]).as_slice()[0];
                let scaled_value = map_to_scaled_value(current_pixel, min, max, color_range);

                let color = colormap.transform_single(scaled_value);
                pixel.copy_from_slice([color.int_r(), color.int_g(), color.int_b()]);
            });

            if self.settings.flip_horizontally {
                imgbuf = imgbuf.run(image_utils::Flip::Horizontal, None);
            }
            if self.settings.flip_vertically {
                imgbuf = imgbuf.run(image_utils::Flip::Vertical, None);
            }

            if self
                .sender
                .send(ProducerMessage::Frame(Frame {
                    image: imgbuf,
                    min: min as f64 / 10.0,
                    max: max as f64 / 10.0,
                }))
                .is_ok()
            {
                self.egui_ctx.request_repaint();
            }
        }
    }

    fn write_emissivity(&mut self) {
        if let Some(rw) = &mut self.rw {
            let mut command: [u8; 4] = [0x55, 0x01, self.settings.emissivity, 0x00];
            let checksum: u8 = command.iter().sum();
            command[3] = checksum;

            if let Err(err) = rw.write_all(&command) {
                log::error!("Failed to write emissivity {err}");
            }
        }
    }

    pub fn main_loop(&mut self) {
        loop {
            self.ensure_port_opened();

            let new_settings: Option<Settings> = {
                let mut received_settings: Option<Settings> = None;

                loop {
                    match self.receiver.try_recv() {
                        Ok(UiMessage::ChangeSettings(settings)) => {
                            received_settings = Some(settings)
                        }
                        Err(TryRecvError::Disconnected) => break received_settings,
                        Err(TryRecvError::Empty) => break received_settings,
                    }
                }
            };

            if let Some(new_settings) = &new_settings {
                self.settings = new_settings.clone();
                self.kernel = self.settings.get_kernel();
                self.write_emissivity();
            }

            if self.rw.is_some() {
                if let Some(gray_image) = self.read_image().as_mut() {
                    self.produce_thermal_frame(gray_image);
                }
            }
        }
    }
}
