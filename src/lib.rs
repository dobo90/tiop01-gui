#[cfg(target_os = "android")]
mod android;
#[cfg(not(target_os = "android"))]
mod unix;

mod app;
mod image_utils;
mod thermal;

use eframe::NativeOptions;

fn _main(native_options: NativeOptions) -> eframe::Result<()> {
    eframe::run_native(
        "Tiop01",
        native_options,
        Box::new(|cc| Box::new(app::App::new(cc))),
    )
}

#[cfg(not(target_os = "android"))]
#[allow(dead_code)]
fn main() -> Result<(), eframe::Error> {
    env_logger::init();

    let native_options = NativeOptions {
        ..eframe::NativeOptions::default()
    };

    _main(native_options)
}

#[cfg(target_os = "android")]
#[no_mangle]
extern "Rust" fn android_main(app: egui_winit::winit::platform::android::activity::AndroidApp) {
    use egui_winit::winit::platform::android::EventLoopBuilderExtAndroid;

    android_logger::init_once(
        android_logger::Config::default().with_max_level(log::LevelFilter::Debug),
    );

    let native_options = NativeOptions {
        event_loop_builder: Some(Box::new(move |builder| {
            builder.with_android_app(app);
        })),
        ..eframe::NativeOptions::default()
    };

    let _ = _main(native_options);
}
