use crate::image_utils;
use crate::thermal::{
    self, ColorMap, EdgeStrategy, FilteringMethod, Frame, ImageProducer, PortOpener, Settings,
    THERMAL_IMAGE_HEIGHT, THERMAL_IMAGE_WIDTH,
};

use std::fmt::Display;

use std::sync::mpsc;
use std::sync::mpsc::{Receiver, Sender};
use std::thread;

use eframe::egui::load::SizedTexture;
use eframe::egui::Ui;
use eframe::egui::{self, TextureOptions};
use strum::IntoEnumIterator;

pub enum ProducerMessage {
    Frame(Frame),
    ConnectionStatusChange(ConnectionStatus),
}

pub enum UiMessage {
    ChangeSettings(Settings),
}

#[derive(PartialEq)]
pub enum ConnectionStatus {
    Disconnected,
    Connected,
}

trait ComboBoxFromIter {
    fn combobox_from_iter<V, I>(&mut self, iter: I, current_value: &mut V, label: &str)
    where
        V: Display + PartialEq,
        I: Iterator<Item = V>;
}

impl ComboBoxFromIter for Ui {
    fn combobox_from_iter<V, I>(&mut self, iter: I, current_value: &mut V, label: &str)
    where
        V: Display + PartialEq,
        I: Iterator<Item = V>,
    {
        egui::ComboBox::from_label(label)
            .selected_text(current_value.to_string())
            .show_ui(self, |ui| {
                for selected_value in iter {
                    let text = selected_value.to_string();
                    ui.selectable_value(current_value, selected_value, text);
                }
            });
    }
}

pub struct App {
    thermal_image_texture: egui::TextureHandle,
    colormap_texture: egui::TextureHandle,
    receiver: Receiver<ProducerMessage>,
    sender: Sender<UiMessage>,
    settings: Settings,
    min: f64,
    max: f64,
    fps: f64,
    last_frame_update: std::time::Instant,
    connection_status: ConnectionStatus,
}

#[cfg(not(target_os = "android"))]
fn producer_main(
    egui_ctx: egui::Context,
    worker_sender: Sender<ProducerMessage>,
    worker_receiver: Receiver<UiMessage>,
) {
    let opener = crate::unix::SerialPortOpener::new();

    producer_main_loop(egui_ctx, worker_sender, worker_receiver, opener);
}

#[cfg(target_os = "android")]
fn producer_main(
    egui_ctx: egui::Context,
    worker_sender: Sender<ProducerMessage>,
    worker_receiver: Receiver<UiMessage>,
) {
    use crate::android::{Context, SerialPortOpener};
    use crate::ANDROID_APP;

    use jni::objects::JObject;
    use std::cell::RefCell;
    use std::rc::Rc;

    let app = ANDROID_APP.get().unwrap();

    let jvm = unsafe { jni::JavaVM::from_raw(app.vm_as_ptr().cast()) }.unwrap();
    let context = unsafe { JObject::from_raw(app.activity_as_ptr().cast()) };
    let env = jvm.attach_current_thread_permanently().unwrap();

    let actx = Context::new(env, context);
    let opener = SerialPortOpener::new(Rc::new(RefCell::new(actx)));

    producer_main_loop(egui_ctx, worker_sender, worker_receiver, opener);
}

fn producer_main_loop<'a, T>(
    egui_ctx: egui::Context,
    worker_sender: Sender<ProducerMessage>,
    worker_receiver: Receiver<UiMessage>,
    opener: T,
) where
    T: PortOpener<'a> + 'a,
{
    let mut producer = ImageProducer::new(egui_ctx, worker_sender, worker_receiver, opener);
    producer.main_loop();
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let egui_ctx = cc.egui_ctx.clone();

        let (ui_sender, worker_receiver): (Sender<UiMessage>, Receiver<UiMessage>) =
            mpsc::channel();
        let (worker_sender, ui_receiver): (Sender<ProducerMessage>, Receiver<ProducerMessage>) =
            mpsc::channel();

        thread::spawn(move || {
            producer_main(egui_ctx, worker_sender, worker_receiver);
        });

        let settings = Settings::default();
        let thermal_image_texture = Self::load_texture_from_black_thermal_image(&cc.egui_ctx);
        let colormap_texture = Self::load_texture_from_colormap_image(
            &cc.egui_ctx,
            &*settings.colormap.get_colormap(),
            settings.color_range,
        );

        Self {
            thermal_image_texture,
            colormap_texture,
            receiver: ui_receiver,
            sender: ui_sender,
            settings,
            min: 0.0,
            max: 0.0,
            fps: 0.0,
            last_frame_update: std::time::Instant::now(),
            connection_status: ConnectionStatus::Disconnected,
        }
    }

    fn receive_producer_message(&mut self) -> Option<ProducerMessage> {
        self.receiver.try_recv().ok()
    }

    fn load_texture_from_image(
        ctx: &egui::Context,
        name: &str,
        image: &image2::Image<u8, image2::Rgb>,
    ) -> egui::TextureHandle {
        let ci = egui::ColorImage::from_rgb(image.size().into(), image.data());
        ctx.load_texture(name, ci, TextureOptions::default())
    }

    fn load_texture_from_black_thermal_image(ctx: &egui::Context) -> egui::TextureHandle {
        let image = image_utils::generate_black_image(THERMAL_IMAGE_WIDTH, THERMAL_IMAGE_HEIGHT);
        Self::load_texture_from_image(ctx, "thermal_image", &image)
    }

    fn load_texture_from_colormap_image(
        ctx: &egui::Context,
        cmap: &(dyn scarlet::colormap::ColorMap<scarlet::color::RGBColor> + Sync),
        color_range: u8,
    ) -> egui::TextureHandle {
        let image = image_utils::generate_colormap_image(256, 1, cmap, color_range);
        Self::load_texture_from_image(ctx, "colormap", &image)
    }

    fn regenerate_colormap(&mut self, ctx: &egui::Context, color_range: u8) {
        self.colormap_texture = Self::load_texture_from_colormap_image(
            ctx,
            &*self.settings.colormap.get_colormap(),
            color_range,
        );
    }

    fn images(&self, ui: &mut Ui) {
        let x = ui.available_size().x;

        ui.image(SizedTexture {
            id: self.thermal_image_texture.id(),
            size: [x, x].into(),
        });

        ui.image(SizedTexture {
            id: self.colormap_texture.id(),
            size: [x, x / 10.0].into(),
        });
    }

    fn settings(&mut self, ui: &mut Ui) {
        egui::widgets::global_dark_light_mode_buttons(ui);
        ui.checkbox(&mut self.settings.flip_vertically, "Flip vertically");
        ui.checkbox(&mut self.settings.flip_horizontally, "Flip horizontally");

        ui.combobox_from_iter(
            FilteringMethod::iter(),
            &mut self.settings.filtering_method,
            "Filtering method",
        );
        ui.combobox_from_iter(
            EdgeStrategy::iter(),
            &mut self.settings.edge_strategy,
            "Edge strategy",
        );
        ui.combobox_from_iter(ColorMap::iter(), &mut self.settings.colormap, "Color map");
        ui.add(
            egui::Slider::new(&mut self.settings.emissivity, 10..=100)
                .prefix("0.")
                .text("Emissivity"),
        );
        ui.add(
            egui::Slider::new(&mut self.settings.color_range, 0..=100)
                .suffix("%")
                .text("Color range"),
        );
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        let screen_size = ctx.screen_rect();
        let use_panels = 1.5 * screen_size.width() > screen_size.height();

        let old_settings = self.settings.clone();
        let message = self.receive_producer_message();
        let mut image: Option<thermal::RgbImage> = None;

        if let Some(message) = message {
            match message {
                ProducerMessage::ConnectionStatusChange(status) => {
                    self.connection_status = status;

                    if self.connection_status == ConnectionStatus::Disconnected {
                        image = Some(image_utils::generate_black_image(
                            THERMAL_IMAGE_WIDTH,
                            THERMAL_IMAGE_HEIGHT,
                        ));
                    }
                }
                ProducerMessage::Frame(frame) => {
                    let now = std::time::Instant::now();
                    self.min = frame.min;
                    self.max = frame.max;
                    self.fps = 1.0 / (now - self.last_frame_update).as_secs_f64();
                    self.last_frame_update = now;
                    image = Some(frame.image);
                }
            }
        }

        if let Some(image) = image {
            self.thermal_image_texture =
                Self::load_texture_from_image(ctx, "thermal_image", &image);
        }

        egui::TopBottomPanel::top("top_panel").show(ctx, |ui| {
            ui.vertical_centered(|ui| {
                ui.heading("Tiop01 thermal camera GUI");
            });
        });

        egui::TopBottomPanel::bottom("bottom_panel").show(ctx, |ui| {
            let text: String = match self.connection_status {
                ConnectionStatus::Disconnected => "Disconnected".into(),
                ConnectionStatus::Connected => format!(
                    "Min: {:.02}, max: {:.02}, FPS: {:.02}",
                    self.min, self.max, self.fps
                ),
            };

            ui.vertical_centered(|ui| {
                ui.label(text);
            });
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            if use_panels {
                ui.columns(2, |columns| {
                    self.images(&mut columns[0]);
                    self.settings(&mut columns[1]);
                });
            } else {
                self.images(ui);
                self.settings(ui);
            }
        });

        if old_settings != self.settings {
            let _ = self
                .sender
                .send(UiMessage::ChangeSettings(self.settings.clone()));

            if old_settings.colormap != self.settings.colormap
                || old_settings.color_range != self.settings.color_range
            {
                self.regenerate_colormap(ctx, self.settings.color_range);
            }
        }
    }
}
