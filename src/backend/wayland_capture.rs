//! One-shot Wayland output capture using the staging ext-image-copy-capture protocol.

use crate::image::{ImageLimits, RgbaImage};
use smithay_client_toolkit::{
    delegate_shm,
    shm::{Shm, ShmHandler, raw::RawPool},
};
use std::thread;
use std::time::{Duration, Instant};
use wayland_client::{
    Connection, Dispatch, QueueHandle, WEnum,
    globals::{GlobalListContents, registry_queue_init},
    protocol::{wl_buffer, wl_output, wl_registry, wl_shm},
};
use wayland_protocols::ext::{
    image_capture_source::v1::client::{
        ext_image_capture_source_v1::ExtImageCaptureSourceV1,
        ext_output_image_capture_source_manager_v1::ExtOutputImageCaptureSourceManagerV1,
    },
    image_copy_capture::v1::client::{
        ext_image_copy_capture_frame_v1::{self, ExtImageCopyCaptureFrameV1},
        ext_image_copy_capture_manager_v1::{self, ExtImageCopyCaptureManagerV1},
        ext_image_copy_capture_session_v1::{self, ExtImageCopyCaptureSessionV1},
    },
};

const CAPTURE_TIMEOUT: Duration = Duration::from_secs(2);

pub(crate) struct CapturedOutput {
    pub(crate) output_id: u32,
    pub(crate) image: RgbaImage,
}

struct OutputCapture {
    output_id: u32,
    _output: wl_output::WlOutput,
    _source: ExtImageCaptureSourceV1,
    session: ExtImageCopyCaptureSessionV1,
    width: Option<u32>,
    height: Option<u32>,
    formats: Vec<wl_shm::Format>,
    constraints_ready: bool,
    format: Option<wl_shm::Format>,
    pool: Option<RawPool>,
    buffer: Option<wl_buffer::WlBuffer>,
    frame: Option<ExtImageCopyCaptureFrameV1>,
    transform: Option<wl_output::Transform>,
    ready: bool,
}

struct CaptureState {
    shm: Shm,
    outputs: Vec<OutputCapture>,
    error: Option<String>,
}

pub(crate) fn capture_outputs(limits: ImageLimits) -> Result<Vec<CapturedOutput>, String> {
    let conn = Connection::connect_to_env()
        .map_err(|error| format!("could not connect for Wayland capture: {error}"))?;
    let (globals, mut event_queue) = registry_queue_init::<CaptureState>(&conn)
        .map_err(|error| format!("could not read Wayland capture globals: {error}"))?;
    let qh = event_queue.handle();
    let shm = Shm::bind(&globals, &qh).map_err(|_| "Wayland capture requires wl_shm".to_owned())?;
    let source_manager: ExtOutputImageCaptureSourceManagerV1 = globals
        .bind(&qh, 1..=1, ())
        .map_err(|_| "compositor lacks ext-output-image-capture-source-v1".to_owned())?;
    let capture_manager: ExtImageCopyCaptureManagerV1 = globals
        .bind(&qh, 1..=1, ())
        .map_err(|_| "compositor lacks ext-image-copy-capture-v1".to_owned())?;

    let output_globals: Vec<_> = globals
        .contents()
        .clone_list()
        .into_iter()
        .filter(|global| global.interface == "wl_output")
        .collect();
    if output_globals.is_empty() {
        return Err("Wayland compositor reported no outputs to capture".to_owned());
    }

    let mut outputs = Vec::with_capacity(output_globals.len());
    for global in output_globals {
        let output = globals.registry().bind::<wl_output::WlOutput, _, _>(
            global.name,
            global.version.min(4),
            &qh,
            (),
        );
        let source = source_manager.create_source(&output, &qh, ());
        let session = capture_manager.create_session(
            &source,
            ext_image_copy_capture_manager_v1::Options::empty(),
            &qh,
            global.name,
        );
        outputs.push(OutputCapture {
            output_id: global.name,
            _output: output,
            _source: source,
            session,
            width: None,
            height: None,
            formats: Vec::new(),
            constraints_ready: false,
            format: None,
            pool: None,
            buffer: None,
            frame: None,
            transform: None,
            ready: false,
        });
    }

    let mut state = CaptureState {
        shm,
        outputs,
        error: None,
    };
    let deadline = Instant::now() + CAPTURE_TIMEOUT;
    while !state.outputs.iter().all(|output| output.ready) {
        event_queue
            .roundtrip(&mut state)
            .map_err(|error| format!("Wayland capture dispatch failed: {error}"))?;
        if let Some(error) = state.error.take() {
            return Err(error);
        }
        state.start_frames(&qh, limits)?;
        if Instant::now() >= deadline {
            return Err("Wayland desktop capture timed out".to_owned());
        }
        thread::sleep(Duration::from_millis(2));
    }

    state.into_images(limits)
}

impl CaptureState {
    fn output_mut(&mut self, output_id: u32) -> Option<&mut OutputCapture> {
        self.outputs
            .iter_mut()
            .find(|output| output.output_id == output_id)
    }

    fn start_frames(&mut self, qh: &QueueHandle<Self>, limits: ImageLimits) -> Result<(), String> {
        let mut allocated_bytes = self
            .outputs
            .iter()
            .filter_map(|output| output.pool.as_ref())
            .try_fold(0_usize, |total, pool| total.checked_add(pool.len()))
            .ok_or_else(|| "aggregate capture size overflowed".to_owned())?;
        for output in &mut self.outputs {
            if !output.constraints_ready || output.frame.is_some() || output.ready {
                continue;
            }
            let (width, height) = output
                .width
                .zip(output.height)
                .ok_or_else(|| "capture constraints omitted the buffer size".to_owned())?;
            let format = choose_format(&output.formats).ok_or_else(|| {
                "capture session offered no supported 32-bit SHM format".to_owned()
            })?;
            let len = checked_capture_len(width, height, limits)?;
            allocated_bytes = allocated_bytes
                .checked_add(len)
                .filter(|total| *total <= limits.max_bytes)
                .ok_or_else(|| "aggregate capture size exceeds safe limits".to_owned())?;
            let width_i32 = i32::try_from(width)
                .map_err(|_| "capture width exceeds the Wayland protocol range".to_owned())?;
            let height_i32 = i32::try_from(height)
                .map_err(|_| "capture height exceeds the Wayland protocol range".to_owned())?;
            let stride = width_i32
                .checked_mul(4)
                .ok_or_else(|| "capture stride overflowed".to_owned())?;
            let mut pool = RawPool::new(len, &self.shm)
                .map_err(|error| format!("could not allocate capture buffer: {error}"))?;
            let buffer = pool.create_buffer(0, width_i32, height_i32, stride, format, (), qh);
            let frame = output.session.create_frame(qh, output.output_id);
            frame.attach_buffer(&buffer);
            frame.damage_buffer(0, 0, width_i32, height_i32);
            frame.capture();
            output.format = Some(format);
            output.pool = Some(pool);
            output.buffer = Some(buffer);
            output.frame = Some(frame);
        }
        Ok(())
    }

    fn into_images(mut self, limits: ImageLimits) -> Result<Vec<CapturedOutput>, String> {
        let mut captured = Vec::with_capacity(self.outputs.len());
        for mut output in self.outputs.drain(..) {
            if output.transform != Some(wl_output::Transform::Normal) {
                return Err(format!(
                    "unsupported capture transform on output {}: {:?}",
                    output.output_id, output.transform
                ));
            }
            let (width, height) = output
                .width
                .zip(output.height)
                .ok_or_else(|| "capture completed without dimensions".to_owned())?;
            let format = output
                .format
                .ok_or_else(|| "capture completed without a pixel format".to_owned())?;
            let mut pool = output
                .pool
                .take()
                .ok_or_else(|| "capture completed without a SHM buffer".to_owned())?;
            let rgba = shm_to_rgba(pool.mmap(), format)?;
            let image = RgbaImage::from_rgba(width, height, rgba, limits)
                .map_err(|error| format!("captured image is invalid: {error}"))?;
            captured.push(CapturedOutput {
                output_id: output.output_id,
                image,
            });
        }
        Ok(captured)
    }
}

fn choose_format(formats: &[wl_shm::Format]) -> Option<wl_shm::Format> {
    [
        wl_shm::Format::Xrgb8888,
        wl_shm::Format::Argb8888,
        wl_shm::Format::Xbgr8888,
        wl_shm::Format::Abgr8888,
    ]
    .into_iter()
    .find(|candidate| formats.contains(candidate))
}

fn checked_capture_len(width: u32, height: u32, limits: ImageLimits) -> Result<usize, String> {
    if width == 0 || height == 0 || width > limits.max_width || height > limits.max_height {
        return Err("capture dimensions exceed safe limits".to_owned());
    }
    let pixels = u64::from(width)
        .checked_mul(u64::from(height))
        .ok_or_else(|| "capture dimensions overflowed".to_owned())?;
    if pixels > limits.max_pixels {
        return Err("capture pixel count exceeds safe limits".to_owned());
    }
    let bytes = usize::try_from(
        pixels
            .checked_mul(4)
            .ok_or_else(|| "capture byte count overflowed".to_owned())?,
    )
    .map_err(|_| "capture byte count exceeds this platform".to_owned())?;
    if bytes > limits.max_bytes {
        return Err("capture byte count exceeds safe limits".to_owned());
    }
    Ok(bytes)
}

fn shm_to_rgba(bytes: &[u8], format: wl_shm::Format) -> Result<Vec<u8>, String> {
    let mut rgba = Vec::new();
    rgba.try_reserve_exact(bytes.len())
        .map_err(|_| "could not allocate converted capture image".to_owned())?;
    for pixel in bytes.as_chunks::<4>().0 {
        let word = u32::from_ne_bytes(*pixel);
        let (red, green, blue) = match format {
            wl_shm::Format::Argb8888 | wl_shm::Format::Xrgb8888 => (
                ((word >> 16) & 0xff) as u8,
                ((word >> 8) & 0xff) as u8,
                (word & 0xff) as u8,
            ),
            wl_shm::Format::Abgr8888 | wl_shm::Format::Xbgr8888 => (
                (word & 0xff) as u8,
                ((word >> 8) & 0xff) as u8,
                ((word >> 16) & 0xff) as u8,
            ),
            _ => unreachable!("format was filtered before conversion"),
        };
        rgba.extend_from_slice(&[red, green, blue, 255]);
    }
    Ok(rgba)
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for CaptureState {
    fn event(
        _state: &mut Self,
        _proxy: &wl_registry::WlRegistry,
        _event: wl_registry::Event,
        _data: &GlobalListContents,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ExtImageCopyCaptureSessionV1, u32> for CaptureState {
    fn event(
        state: &mut Self,
        _proxy: &ExtImageCopyCaptureSessionV1,
        event: ext_image_copy_capture_session_v1::Event,
        output_id: &u32,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        let Some(output) = state.output_mut(*output_id) else {
            return;
        };
        match event {
            ext_image_copy_capture_session_v1::Event::BufferSize { width, height } => {
                output.width = Some(width);
                output.height = Some(height);
            }
            ext_image_copy_capture_session_v1::Event::ShmFormat {
                format: WEnum::Value(format),
            } => output.formats.push(format),
            ext_image_copy_capture_session_v1::Event::Done => {
                output.constraints_ready = true;
            }
            ext_image_copy_capture_session_v1::Event::Stopped => {
                state.error = Some(format!("capture session stopped for output {output_id}"));
            }
            _ => {}
        }
    }
}

impl Dispatch<ExtImageCopyCaptureFrameV1, u32> for CaptureState {
    fn event(
        state: &mut Self,
        proxy: &ExtImageCopyCaptureFrameV1,
        event: ext_image_copy_capture_frame_v1::Event,
        output_id: &u32,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        let Some(output) = state.output_mut(*output_id) else {
            return;
        };
        match event {
            ext_image_copy_capture_frame_v1::Event::Transform {
                transform: WEnum::Value(transform),
            } => output.transform = Some(transform),
            ext_image_copy_capture_frame_v1::Event::Ready => {
                output.ready = true;
                proxy.destroy();
            }
            ext_image_copy_capture_frame_v1::Event::Failed { reason } => {
                state.error = Some(format!("capture failed for output {output_id}: {reason:?}"));
                proxy.destroy();
            }
            _ => {}
        }
    }
}

impl ShmHandler for CaptureState {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

delegate_shm!(CaptureState);
wayland_client::delegate_noop!(CaptureState: ignore wl_output::WlOutput);
wayland_client::delegate_noop!(CaptureState: ignore wl_buffer::WlBuffer);
wayland_client::delegate_noop!(CaptureState: ignore ExtOutputImageCaptureSourceManagerV1);
wayland_client::delegate_noop!(CaptureState: ignore ExtImageCaptureSourceV1);
wayland_client::delegate_noop!(CaptureState: ignore ExtImageCopyCaptureManagerV1);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xrgb_native_words_convert_to_opaque_rgba() {
        let bytes = 0x0011_2233_u32.to_ne_bytes();
        assert_eq!(
            shm_to_rgba(&bytes, wl_shm::Format::Xrgb8888).unwrap(),
            [0x11, 0x22, 0x33, 0xff]
        );
    }

    #[test]
    fn xbgr_native_words_convert_to_opaque_rgba() {
        let bytes = 0x0033_2211_u32.to_ne_bytes();
        assert_eq!(
            shm_to_rgba(&bytes, wl_shm::Format::Xbgr8888).unwrap(),
            [0x11, 0x22, 0x33, 0xff]
        );
    }
}
