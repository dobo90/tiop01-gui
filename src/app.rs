use crate::image_utils;
use crate::thermal::{
    self, ColorMap, EdgeStrategy, FilteringMethod, Frame, Settings, ThermalImageProducer,
    THERMAL_IMAGE_HEIGHT, THERMAL_IMAGE_WIDTH,
};

use std::fmt::Debug;
use std::ops::Deref;
use std::sync::mpsc;
use std::sync::mpsc::{Receiver, Sender};
use std::thread;

use eframe::egui;
use eframe::egui::load::SizedTexture;
use eframe::egui::Ui;
use strum::IntoEnumIterator;

pub enum ProducerMessage {
    Frame(Frame),
}

pub enum UiMessage {
    ChangeSettings(Settings),
}

trait ComboBoxFromIter {
    fn combobox_from_iter<V, I>(&mut self, iter: I, current_value: &mut V, label: &str)
    where
        V: Debug + PartialEq,
        I: Iterator<Item = V>;
}

impl ComboBoxFromIter for Ui {
    fn combobox_from_iter<V, I>(&mut self, iter: I, current_value: &mut V, label: &str)
    where
        V: Debug + PartialEq,
        I: Iterator<Item = V>,
    {
        egui::ComboBox::from_label(label)
            .selected_text(format!("{:?}", current_value))
            .show_ui(self, |ui| {
                for selected_value in iter {
                    let text = format!("{:?}", &selected_value);
                    ui.selectable_value(current_value, selected_value, text);
                }
            });
    }
}

pub struct Tiop01App {
    thermal_image_texture: Option<egui::TextureHandle>,
    colormap_texture: Option<egui::TextureHandle>,
    receiver: Receiver<ProducerMessage>,
    sender: Sender<UiMessage>,
    settings: Settings,
    min: f64,
    max: f64,
    fps: f64,
    last_frame_update: std::time::Instant,
}

#[cfg(not(target_os = "android"))]
fn producer_main(
    egui_ctx: egui::Context,
    worker_sender: Sender<ProducerMessage>,
    worker_receiver: Receiver<UiMessage>,
) {
    use crate::unix::SerialPortOpener;

    let opener = Box::new(SerialPortOpener::new());

    let mut producer = ThermalImageProducer::new(&egui_ctx, worker_sender, worker_receiver, opener);
    producer.main_loop();
}
#[cfg(target_os = "android")]
fn producer_main(
    egui_ctx: egui::Context,
    worker_sender: Sender<ProducerMessage>,
    worker_receiver: Receiver<UiMessage>,
) {
    use crate::android::{AndroidCtx, SerialPortOpener};
    use jni::objects::JObject;

    use std::cell::RefCell;
    use std::rc::Rc;

    let ctx = ndk_context::android_context();
    let jvm = unsafe { jni::JavaVM::from_raw(ctx.vm().cast()) }.unwrap();
    let context = unsafe { JObject::from_raw(ctx.context().cast()) };
    let env = jvm.attach_current_thread_permanently().unwrap();

    let actx = AndroidCtx::new(env, context);
    let opener = Box::new(SerialPortOpener::new(Rc::new(RefCell::new(actx))));

    let mut producer = ThermalImageProducer::new(&egui_ctx, worker_sender, worker_receiver, opener);
    producer.main_loop();
}

impl Tiop01App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let egui_ctx = cc.egui_ctx.clone();

        let (ui_sender, worker_receiver): (Sender<UiMessage>, Receiver<UiMessage>) =
            mpsc::channel();
        let (worker_sender, ui_receiver): (Sender<ProducerMessage>, Receiver<ProducerMessage>) =
            mpsc::channel();

        thread::spawn(move || {
            producer_main(egui_ctx, worker_sender, worker_receiver);
        });

        Self {
            thermal_image_texture: None,
            colormap_texture: None,
            receiver: ui_receiver,
            sender: ui_sender,
            settings: Settings::default(),
            min: 0.0,
            max: 0.0,
            fps: 0.0,
            last_frame_update: std::time::Instant::now(),
        }
    }

    fn receive_frame(&mut self) -> Option<thermal::RgbImage> {
        if let Ok(ProducerMessage::Frame(frame)) = self.receiver.try_recv() {
            let now = std::time::Instant::now();
            self.min = frame.min;
            self.max = frame.max;
            self.fps = 1.0 / (now - self.last_frame_update).as_secs_f64();
            self.last_frame_update = now;
            Some(frame.image)
        } else if self.thermal_image_texture.is_none() {
            Some(image_utils::generate_black_image(
                THERMAL_IMAGE_WIDTH,
                THERMAL_IMAGE_HEIGHT,
            ))
        } else {
            None
        }
    }

    fn regenerate_colormap(&mut self, ctx: &egui::Context) {
        let image = image_utils::generate_colormap_image(
            256,
            1,
            self.settings.colormap.get_colormap().deref(),
        );
        let ci = egui::ColorImage::from_rgb(image.size().into(), image.data());
        self.colormap_texture = Some(ctx.load_texture("colormap", ci, Default::default()));
    }

    fn images(&self, ui: &mut Ui) {
        let x = ui.available_size().x;

        ui.image(SizedTexture {
            id: self.thermal_image_texture.as_ref().unwrap().id(),
            size: [x, x].into(),
        });
        ui.image(SizedTexture {
            id: self.colormap_texture.as_ref().unwrap().id(),
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
    }
}

impl eframe::App for Tiop01App {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        let window_size = frame.info().window_info.size;
        let use_panels = 1.5 * window_size.x > window_size.y;
        let old_settings = self.settings.clone();

        let image = self.receive_frame();

        if let Some(image) = image {
            let ci = egui::ColorImage::from_rgb(image.size().into(), image.data());
            self.thermal_image_texture =
                Some(ctx.load_texture("thermal_image", ci, Default::default()));
        }

        if self.colormap_texture.is_none() {
            self.regenerate_colormap(ctx);
        }

        egui::TopBottomPanel::top("top_panel").show(ctx, |ui| {
            ui.vertical_centered(|ui| {
                ui.heading("Tiop01 thermal camera GUI");
            });
        });

        egui::TopBottomPanel::bottom("bottom_panel").show(ctx, |ui| {
            ui.vertical_centered(|ui| {
                ui.label(format!(
                    "Min: {:.02}, max: {:.02}, FPS: {:.02}",
                    self.min, self.max, self.fps
                ));
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

            if old_settings.colormap != self.settings.colormap {
                self.regenerate_colormap(ctx);
            }
        }
    }
}
