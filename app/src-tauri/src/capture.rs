//! Screen capture implementation

#![allow(dead_code)]
#![allow(unused_imports)]

use anyhow::Result;
use std::sync::atomic::{AtomicU8, AtomicU32, Ordering};

/// Global quality setting (1-100, default 75)
static JPEG_QUALITY: AtomicU8 = AtomicU8::new(75);

/// Frame counter for statistics
static FRAME_COUNT: AtomicU32 = AtomicU32::new(0);

/// Set the JPEG encoding quality (1-100)
pub fn set_quality(quality: u8) {
    JPEG_QUALITY.store(quality.clamp(1, 100), Ordering::Relaxed);
}

/// Get current quality setting
pub fn get_quality() -> u8 {
    JPEG_QUALITY.load(Ordering::Relaxed)
}

/// Get frame count for statistics
pub fn get_frame_count() -> u32 {
    FRAME_COUNT.load(Ordering::Relaxed)
}

#[cfg(windows)]
mod windows_capture {
    use super::*;
    use anyhow::{Context, Result};
    use windows::{
        core::*,
        Win32::Foundation::RECT,
        Win32::Graphics::Direct3D::*,
        Win32::Graphics::Direct3D11::*,
        Win32::Graphics::Dxgi::Common::*,
        Win32::Graphics::Dxgi::*,
    };

    pub struct ScreenCapture {
        device: ID3D11Device,
        context: ID3D11DeviceContext,
        duplication: IDXGIOutputDuplication,
        staging: ID3D11Texture2D,
        width: u32,
        height: u32,
        last_frame: Option<Vec<u8>>,
        unchanged_count: u32,
        // Track if duplication needs recreation
        needs_recreate: bool,
    }

    impl ScreenCapture {
        pub fn new() -> Result<Self> {
            unsafe { Self::init() }
        }

        unsafe fn init() -> Result<Self> {
            let mut device: Option<ID3D11Device> = None;
            let mut context: Option<ID3D11DeviceContext> = None;

            D3D11CreateDevice(
                None,
                D3D_DRIVER_TYPE_HARDWARE,
                None,
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                Some(&[D3D_FEATURE_LEVEL_11_0]),
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                Some(&mut context),
            )?;

            let device = device.context("No D3D11 device")?;
            let context = context.context("No D3D11 context")?;

            let dxgi_device: IDXGIDevice = device.cast()?;
            let adapter: IDXGIAdapter = dxgi_device.GetAdapter()?;
            let output: IDXGIOutput = adapter.EnumOutputs(0)?;
            let output1: IDXGIOutput1 = output.cast()?;

            let mut desc = DXGI_OUTPUT_DESC::default();
            output.GetDesc(&mut desc)?;
            let width = (desc.DesktopCoordinates.right - desc.DesktopCoordinates.left) as u32;
            let height = (desc.DesktopCoordinates.bottom - desc.DesktopCoordinates.top) as u32;

            let duplication = output1.DuplicateOutput(&device)?;

            let tex_desc = D3D11_TEXTURE2D_DESC {
                Width: width,
                Height: height,
                MipLevels: 1,
                ArraySize: 1,
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                Usage: D3D11_USAGE_STAGING,
                BindFlags: 0,
                CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
                MiscFlags: 0,
            };

            let mut staging: Option<ID3D11Texture2D> = None;
            device.CreateTexture2D(&tex_desc, None, Some(&mut staging))?;
            let staging = staging.context("No staging texture")?;

            Ok(Self {
                device,
                context,
                duplication,
                staging,
                width,
                height,
                last_frame: None,
                unchanged_count: 0,
                needs_recreate: false,
            })
        }

        /// Recreate duplication output (needed after display changes, UAC prompts, etc.)
        unsafe fn recreate_duplication(&mut self) -> Result<()> {
            let dxgi_device: IDXGIDevice = self.device.cast()?;
            let adapter: IDXGIAdapter = dxgi_device.GetAdapter()?;
            let output: IDXGIOutput = adapter.EnumOutputs(0)?;
            let output1: IDXGIOutput1 = output.cast()?;

            self.duplication = output1.DuplicateOutput(&self.device)?;
            self.needs_recreate = false;
            Ok(())
        }

        pub fn capture(&mut self) -> Result<(u32, u32, Vec<u8>)> {
            unsafe { self.capture_internal() }
        }

        unsafe fn capture_internal(&mut self) -> Result<(u32, u32, Vec<u8>)> {
            // Recreate duplication if needed
            if self.needs_recreate {
                if let Err(e) = self.recreate_duplication() {
                    println!("[CAPTURE] Failed to recreate duplication: {}", e);
                    // Return last frame if available
                    if let Some(ref frame) = self.last_frame {
                        return Ok((self.width, self.height, frame.clone()));
                    }
                    return Err(e);
                }
            }

            let mut frame_info = DXGI_OUTDUPL_FRAME_INFO::default();
            let mut resource: Option<IDXGIResource> = None;

            match self.duplication.AcquireNextFrame(100, &mut frame_info, &mut resource) {
                Ok(()) => {}
                Err(e) if e.code() == DXGI_ERROR_WAIT_TIMEOUT => {
                    // No new frame - return cached frame if available
                    if let Some(ref frame) = self.last_frame {
                        self.unchanged_count += 1;
                        if self.unchanged_count <= 10 {
                            return Ok((self.width, self.height, frame.clone()));
                        }
                    }
                    return Ok((self.width, self.height, Vec::new()));
                }
                Err(e) if e.code() == DXGI_ERROR_ACCESS_LOST => {
                    // Display mode changed or UAC prompt - need to recreate
                    self.needs_recreate = true;
                    if let Some(ref frame) = self.last_frame {
                        return Ok((self.width, self.height, frame.clone()));
                    }
                    return Err(e.into());
                }
                Err(e) => return Err(e.into()),
            }

            // Reset unchanged counter since we have a new frame
            self.unchanged_count = 0;

            let resource = resource.context("No resource")?;
            let texture: ID3D11Texture2D = resource.cast()?;

            self.context.CopyResource(&self.staging, &texture);

            let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
            self.context.Map(&self.staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))?;

            let pitch = mapped.RowPitch as usize;
            let data = std::slice::from_raw_parts(
                mapped.pData as *const u8,
                pitch * self.height as usize,
            );

            // Convert BGRA to RGB and encode as JPEG with adaptive quality
            let rgb = self.bgra_to_rgb(data, pitch);
            let jpeg = self.encode_jpeg(&rgb)?;

            self.context.Unmap(&self.staging, 0);
            self.duplication.ReleaseFrame()?;

            // Cache the frame for reuse
            self.last_frame = Some(jpeg.clone());

            // Update frame counter
            FRAME_COUNT.fetch_add(1, Ordering::Relaxed);

            Ok((self.width, self.height, jpeg))
        }

        fn bgra_to_rgb(&self, bgra: &[u8], pitch: usize) -> Vec<u8> {
            let mut rgb = Vec::with_capacity((self.width * self.height * 3) as usize);
            for y in 0..self.height as usize {
                for x in 0..self.width as usize {
                    let i = y * pitch + x * 4;
                    rgb.push(bgra[i + 2]); // R
                    rgb.push(bgra[i + 1]); // G
                    rgb.push(bgra[i]);     // B
                }
            }
            rgb
        }

        fn encode_jpeg(&self, rgb: &[u8]) -> Result<Vec<u8>> {
            use image::codecs::jpeg::JpegEncoder;
            use image::ColorType;

            let quality = JPEG_QUALITY.load(Ordering::Relaxed);

            let mut jpeg = Vec::new();
            {
                let mut encoder = JpegEncoder::new_with_quality(&mut jpeg, quality);
                encoder.encode(
                    rgb,
                    self.width,
                    self.height,
                    ColorType::Rgb8,
                )?;
            }

            Ok(jpeg)
        }
    }
}

#[cfg(windows)]
pub use windows_capture::ScreenCapture;

#[cfg(target_os = "macos")]
mod macos_capture {
    use super::*;
    use anyhow::{Context, Result};
    use core_graphics::display::{CGDisplay, CGMainDisplayID};
    use std::sync::atomic::Ordering;

    pub struct ScreenCapture {
        display_id: u32,
        width: u32,
        height: u32,
        last_frame: Option<Vec<u8>>,
    }

    impl ScreenCapture {
        pub fn new() -> Result<Self> {
            let display_id = unsafe { CGMainDisplayID() };
            let display = CGDisplay::new(display_id);

            let width = display.pixels_wide() as u32;
            let height = display.pixels_high() as u32;

            println!("[CAPTURE] macOS display: {}x{}", width, height);

            Ok(Self {
                display_id,
                width,
                height,
                last_frame: None,
            })
        }

        pub fn capture(&mut self) -> Result<(u32, u32, Vec<u8>)> {
            use core_graphics::display::CGDisplayCreateImage;

            // Create image from display
            let image = unsafe { CGDisplayCreateImage(self.display_id) };

            if image.is_null() {
                // Return last frame if capture failed
                if let Some(ref frame) = self.last_frame {
                    return Ok((self.width, self.height, frame.clone()));
                }
                anyhow::bail!("Failed to capture screen - check Screen Recording permission");
            }

            // Get image dimensions and data
            let width: usize;
            let height: usize;
            let bytes_per_row: usize;
            let pixel_data: Vec<u8>;

            unsafe {
                use core_foundation::base::TCFType;
                use core_graphics::image::CGImage;

                let cg_image = CGImage::wrap_under_create_rule(image);
                width = cg_image.width();
                height = cg_image.height();
                bytes_per_row = cg_image.bytes_per_row();

                // Get pixel data from CGImage
                if let Some(data_provider) = cg_image.data_provider() {
                    let data = data_provider.copy_data();
                    pixel_data = data.bytes().to_vec();
                } else {
                    if let Some(ref frame) = self.last_frame {
                        return Ok((self.width, self.height, frame.clone()));
                    }
                    anyhow::bail!("No data provider");
                }
            }

            // Convert BGRA/RGBA to RGB
            let rgb = self.convert_to_rgb(&pixel_data, bytes_per_row, width, height);

            // Encode as JPEG
            let jpeg = self.encode_jpeg(&rgb, width as u32, height as u32)?;

            // Cache frame
            self.last_frame = Some(jpeg.clone());
            self.width = width as u32;
            self.height = height as u32;

            // Update frame counter
            FRAME_COUNT.fetch_add(1, Ordering::Relaxed);

            Ok((self.width, self.height, jpeg))
        }

        fn convert_to_rgb(&self, pixels: &[u8], bytes_per_row: usize, width: usize, height: usize) -> Vec<u8> {
            let mut rgb = Vec::with_capacity(width * height * 3);

            for y in 0..height {
                for x in 0..width {
                    let i = y * bytes_per_row + x * 4;
                    if i + 3 < pixels.len() {
                        // macOS uses BGRA format
                        rgb.push(pixels[i + 2]); // R
                        rgb.push(pixels[i + 1]); // G
                        rgb.push(pixels[i]);     // B
                    }
                }
            }
            rgb
        }

        fn encode_jpeg(&self, rgb: &[u8], width: u32, height: u32) -> Result<Vec<u8>> {
            use image::codecs::jpeg::JpegEncoder;
            use image::ColorType;

            let quality = JPEG_QUALITY.load(Ordering::Relaxed);

            let mut jpeg = Vec::new();
            {
                let mut encoder = JpegEncoder::new_with_quality(&mut jpeg, quality);
                encoder.encode(rgb, width, height, ColorType::Rgb8)?;
            }

            Ok(jpeg)
        }
    }
}

#[cfg(target_os = "macos")]
pub use macos_capture::ScreenCapture;

#[cfg(target_os = "linux")]
mod linux_capture {
    use super::*;
    use anyhow::Result;
    use std::ptr;
    use std::sync::atomic::Ordering;
    use x11::xlib::*;

    /// AllPlanes constant - returns all bits set (equivalent to !0)
    /// This is the value that XAllPlanes() returns
    #[inline]
    fn all_planes() -> u64 {
        !0u64
    }

    pub struct ScreenCapture {
        display: *mut Display,
        root: Window,
        width: u32,
        height: u32,
        last_frame: Option<Vec<u8>>,
    }

    // Display pointer is thread-safe for our use case
    unsafe impl Send for ScreenCapture {}
    unsafe impl Sync for ScreenCapture {}

    impl ScreenCapture {
        pub fn new() -> Result<Self> {
            unsafe {
                let display = XOpenDisplay(ptr::null());
                if display.is_null() {
                    anyhow::bail!("Failed to open X11 display - is DISPLAY set?");
                }

                let screen = XDefaultScreen(display);
                let root = XRootWindow(display, screen);
                let width = XDisplayWidth(display, screen) as u32;
                let height = XDisplayHeight(display, screen) as u32;

                println!("[CAPTURE] Linux X11 display: {}x{}", width, height);

                Ok(Self {
                    display,
                    root,
                    width,
                    height,
                    last_frame: None,
                })
            }
        }

        pub fn capture(&mut self) -> Result<(u32, u32, Vec<u8>)> {
            unsafe { self.capture_x11() }
        }

        unsafe fn capture_x11(&mut self) -> Result<(u32, u32, Vec<u8>)> {
            // Use XGetImage (slower but always works)
            // all_planes() returns !0 which is equivalent to XAllPlanes()
            let image = XGetImage(
                self.display,
                self.root,
                0,
                0,
                self.width,
                self.height,
                all_planes(),
                ZPixmap,
            );

            if image.is_null() {
                if let Some(ref frame) = self.last_frame {
                    return Ok((self.width, self.height, frame.clone()));
                }
                anyhow::bail!("Failed to capture screen");
            }

            let rgb = self.ximage_to_rgb(image);
            XDestroyImage(image);

            let jpeg = self.encode_jpeg(&rgb)?;

            self.last_frame = Some(jpeg.clone());
            FRAME_COUNT.fetch_add(1, Ordering::Relaxed);

            Ok((self.width, self.height, jpeg))
        }

        unsafe fn ximage_to_rgb(&self, image: *mut XImage) -> Vec<u8> {
            let width = (*image).width as usize;
            let height = (*image).height as usize;
            let bytes_per_line = (*image).bytes_per_line as usize;
            let bits_per_pixel = (*image).bits_per_pixel;
            let data = (*image).data as *const u8;

            let mut rgb = Vec::with_capacity(width * height * 3);

            for y in 0..height {
                for x in 0..width {
                    let pixel_offset = y * bytes_per_line + x * (bits_per_pixel as usize / 8);
                    let pixel = data.add(pixel_offset);

                    // X11 typically uses BGRA or BGR format
                    if bits_per_pixel == 32 {
                        rgb.push(*pixel.add(2)); // R
                        rgb.push(*pixel.add(1)); // G
                        rgb.push(*pixel.add(0)); // B
                    } else if bits_per_pixel == 24 {
                        rgb.push(*pixel.add(2)); // R
                        rgb.push(*pixel.add(1)); // G
                        rgb.push(*pixel.add(0)); // B
                    } else {
                        // Fallback for other formats
                        rgb.push(0);
                        rgb.push(0);
                        rgb.push(0);
                    }
                }
            }
            rgb
        }

        fn encode_jpeg(&self, rgb: &[u8]) -> Result<Vec<u8>> {
            use image::codecs::jpeg::JpegEncoder;
            use image::ColorType;

            let quality = JPEG_QUALITY.load(Ordering::Relaxed);

            let mut jpeg = Vec::new();
            {
                let mut encoder = JpegEncoder::new_with_quality(&mut jpeg, quality);
                encoder.encode(rgb, self.width, self.height, ColorType::Rgb8)?;
            }

            Ok(jpeg)
        }
    }

    impl Drop for ScreenCapture {
        fn drop(&mut self) {
            unsafe {
                if !self.display.is_null() {
                    XCloseDisplay(self.display);
                }
            }
        }
    }
}

#[cfg(target_os = "linux")]
pub use linux_capture::ScreenCapture;

// Stub for unsupported platforms
#[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
pub struct ScreenCapture;

#[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
impl ScreenCapture {
    pub fn new() -> Result<Self> {
        Ok(Self)
    }

    pub fn capture(&mut self) -> Result<(u32, u32, Vec<u8>)> {
        Ok((1920, 1080, Vec::new()))
    }
}
