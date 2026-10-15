//! Shared static-background loading for the display backends.

use crate::image::{FitMode, ImageEffect, ImageLimits, RgbaImage, load_png, load_png_bytes};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};
use std::thread;

pub const LOCK_COLOR: [u8; 4] = [18, 18, 20, 255];
const FALLBACK_PNG: &[u8] = include_bytes!("../assets/amorphis-fallback.png");

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BackgroundSource {
    File(PathBuf),
    EmbeddedFallback,
    DesktopCapture,
}

#[derive(Clone, Debug)]
pub struct StaticBackground {
    pub source: BackgroundSource,
    pub fit: FitMode,
    pub effect: ImageEffect,
}

pub type BackgroundReceiver = Receiver<Result<RgbaImage, String>>;

/// Load and process the selected PNG away from the display event loop.
///
/// Callers can establish an opaque solid lock immediately and replace it once
/// the worker returns a complete image.
pub fn start_loader(
    background: Option<StaticBackground>,
    limits: ImageLimits,
) -> (Option<BackgroundReceiver>, FitMode) {
    let Some(background) = background else {
        return (None, FitMode::Cover);
    };
    let fit = background.fit;
    let (sender, receiver) = mpsc::sync_channel(1);
    let spawn = thread::Builder::new()
        .name("amorphis-image".into())
        .spawn(move || {
            let result = load_background(&background.source, limits).and_then(|mut image| {
                background
                    .effect
                    .apply(&mut image, limits)
                    .map_err(|error| format!("could not apply image effect: {error}"))?;
                Ok(image)
            });
            let _ = sender.send(result);
        });
    if let Err(error) = spawn {
        eprintln!("amorphis: could not start PNG loader: {error}; using solid background");
        return (None, fit);
    }
    (Some(receiver), fit)
}

pub(crate) fn load_background(
    source: &BackgroundSource,
    limits: ImageLimits,
) -> Result<RgbaImage, String> {
    match source {
        BackgroundSource::DesktopCapture => {
            Err("desktop capture requires backend support".to_owned())
        }
        BackgroundSource::EmbeddedFallback => {
            load_png_bytes(FALLBACK_PNG, limits).map_err(|error| error.to_string())
        }
        BackgroundSource::File(path) => match load_png(path, limits) {
            Ok(image) => Ok(image),
            Err(primary_error) => {
                eprintln!("amorphis: {primary_error}; using the embedded fallback background");
                load_png_bytes(FALLBACK_PNG, limits).map_err(|fallback_error| {
                    format!(
                        "custom PNG failed ({primary_error}) and embedded fallback failed ({fallback_error})"
                    )
                })
            }
        },
    }
}
