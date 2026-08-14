use std::ffi::c_void;
use std::thread;
use std::time::{Duration, Instant};

use windows::core::{factory, Interface};
use windows::Graphics::Capture::{
    Direct3D11CaptureFrame, Direct3D11CaptureFramePool, GraphicsCaptureItem, GraphicsCaptureSession,
};
use windows::Graphics::DirectX::Direct3D11::IDirect3DDevice;
use windows::Graphics::DirectX::DirectXPixelFormat;
use windows::Win32::Foundation::{HMODULE, HWND, RPC_E_CHANGED_MODE};
use windows::Win32::Graphics::Direct3D::{
    D3D_DRIVER_TYPE, D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_WARP,
};
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D, D3D11_CPU_ACCESS_READ,
    D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_MAPPED_SUBRESOURCE, D3D11_MAP_READ, D3D11_SDK_VERSION,
    D3D11_TEXTURE2D_DESC, D3D11_USAGE_STAGING,
};
use windows::Win32::Graphics::Dxgi::IDXGIDevice;
use windows::Win32::System::WinRT::Direct3D11::{
    CreateDirect3D11DeviceFromDXGIDevice, IDirect3DDxgiInterfaceAccess,
};
use windows::Win32::System::WinRT::Graphics::Capture::IGraphicsCaptureItemInterop;
use windows::Win32::System::WinRT::{RoInitialize, RoUninitialize, RO_INIT_MULTITHREADED};

const FRAME_TIMEOUT: Duration = Duration::from_secs(3);
const FRAME_POLL_INTERVAL: Duration = Duration::from_millis(8);

pub(super) struct CapturedRgbaFrame {
    pub pixels: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

struct RoApartment {
    owns_initialization: bool,
}

impl Drop for RoApartment {
    fn drop(&mut self) {
        if self.owns_initialization {
            unsafe { RoUninitialize() };
        }
    }
}

pub(super) fn capture_window(hwnd: isize) -> Result<CapturedRgbaFrame, String> {
    let _apartment = initialize_runtime()?;
    if !GraphicsCaptureSession::IsSupported()
        .map_err(|error| format!("failed to query Windows Graphics Capture support: {error}"))?
    {
        return Err("Windows Graphics Capture is unsupported on this system".to_string());
    }

    let (d3d_device, d3d_context, capture_device) = create_capture_device()?;
    let interop: IGraphicsCaptureItemInterop = factory::<GraphicsCaptureItem, _>()
        .map_err(|error| format!("failed to acquire GraphicsCaptureItem factory: {error}"))?;
    let item: GraphicsCaptureItem =
        unsafe { interop.CreateForWindow(HWND(hwnd as *mut c_void)) }
            .map_err(|error| format!("failed to create capture item for window: {error}"))?;
    let size = item
        .Size()
        .map_err(|error| format!("failed to read capture item size: {error}"))?;
    if size.Width <= 0 || size.Height <= 0 {
        return Err("capture item reported an empty frame size".to_string());
    }

    let frame_pool = Direct3D11CaptureFramePool::CreateFreeThreaded(
        &capture_device,
        DirectXPixelFormat::B8G8R8A8UIntNormalized,
        2,
        size,
    )
    .map_err(|error| format!("failed to create capture frame pool: {error}"))?;
    let session = frame_pool
        .CreateCaptureSession(&item)
        .map_err(|error| format!("failed to create capture session: {error}"))?;
    // Cursor capture is irrelevant for observations and can obscure UI details. The setter was
    // added after the base capture API, so an older Windows build may legitimately reject it.
    let _ = session.SetIsCursorCaptureEnabled(false);
    session
        .StartCapture()
        .map_err(|error| format!("failed to start capture session: {error}"))?;

    let result = (|| {
        let frame = wait_for_frame(&frame_pool)?;
        let pixels = read_frame(&d3d_device, &d3d_context, &frame);
        let _ = frame.Close();
        pixels
    })();

    let _ = session.Close();
    let _ = frame_pool.Close();
    result
}

fn initialize_runtime() -> Result<RoApartment, String> {
    match unsafe { RoInitialize(RO_INIT_MULTITHREADED) } {
        Ok(()) => Ok(RoApartment {
            owns_initialization: true,
        }),
        // A reused blocking thread may already belong to an STA. WinRT is initialized in that
        // case, so capture can proceed, but this call must not balance someone else's apartment.
        Err(error) if error.code() == RPC_E_CHANGED_MODE => Ok(RoApartment {
            owns_initialization: false,
        }),
        Err(error) => Err(format!("failed to initialize WinRT: {error}")),
    }
}

fn create_capture_device() -> Result<(ID3D11Device, ID3D11DeviceContext, IDirect3DDevice), String> {
    let (device, context) =
        create_d3d11_device(D3D_DRIVER_TYPE_HARDWARE).or_else(|hardware_error| {
            create_d3d11_device(D3D_DRIVER_TYPE_WARP).map_err(|warp_error| {
                format!(
                    "failed to create D3D11 device (hardware: {hardware_error}; WARP: {warp_error})"
                )
            })
        })?;
    let dxgi_device: IDXGIDevice = device
        .cast()
        .map_err(|error| format!("failed to query IDXGIDevice: {error}"))?;
    let inspectable = unsafe { CreateDirect3D11DeviceFromDXGIDevice(&dxgi_device) }
        .map_err(|error| format!("failed to create WinRT D3D11 device: {error}"))?;
    let capture_device: IDirect3DDevice = inspectable
        .cast()
        .map_err(|error| format!("failed to query IDirect3DDevice: {error}"))?;
    Ok((device, context, capture_device))
}

fn create_d3d11_device(
    driver_type: D3D_DRIVER_TYPE,
) -> Result<(ID3D11Device, ID3D11DeviceContext), String> {
    let mut device = None;
    let mut context = None;
    unsafe {
        D3D11CreateDevice(
            None,
            driver_type,
            HMODULE::default(),
            D3D11_CREATE_DEVICE_BGRA_SUPPORT,
            None,
            D3D11_SDK_VERSION,
            Some(&mut device),
            None,
            Some(&mut context),
        )
    }
    .map_err(|error| error.to_string())?;
    let device = device.ok_or_else(|| "D3D11CreateDevice returned no device".to_string())?;
    let context = context.ok_or_else(|| "D3D11CreateDevice returned no context".to_string())?;
    Ok((device, context))
}

fn wait_for_frame(
    frame_pool: &Direct3D11CaptureFramePool,
) -> Result<Direct3D11CaptureFrame, String> {
    let deadline = Instant::now() + FRAME_TIMEOUT;
    loop {
        match frame_pool.TryGetNextFrame() {
            Ok(frame) => return Ok(frame),
            Err(error) if Instant::now() >= deadline => {
                return Err(format!("timed out waiting for a capture frame: {error}"));
            }
            Err(_) => {}
        }
        thread::sleep(FRAME_POLL_INTERVAL);
    }
}

fn read_frame(
    device: &ID3D11Device,
    context: &ID3D11DeviceContext,
    frame: &Direct3D11CaptureFrame,
) -> Result<CapturedRgbaFrame, String> {
    let content_size = frame
        .ContentSize()
        .map_err(|error| format!("failed to read captured frame size: {error}"))?;
    if content_size.Width <= 0 || content_size.Height <= 0 {
        return Err("captured frame reported an empty content size".to_string());
    }

    let surface = frame
        .Surface()
        .map_err(|error| format!("failed to read captured surface: {error}"))?;
    let access: IDirect3DDxgiInterfaceAccess = surface
        .cast()
        .map_err(|error| format!("failed to access captured DXGI surface: {error}"))?;
    let texture: ID3D11Texture2D = unsafe { access.GetInterface() }
        .map_err(|error| format!("failed to access captured D3D11 texture: {error}"))?;

    let mut source_desc = D3D11_TEXTURE2D_DESC::default();
    unsafe { texture.GetDesc(&mut source_desc) };
    let width = (content_size.Width as u32).min(source_desc.Width);
    let height = (content_size.Height as u32).min(source_desc.Height);
    if width == 0 || height == 0 {
        return Err("captured texture has empty dimensions".to_string());
    }

    let staging_desc = D3D11_TEXTURE2D_DESC {
        Width: source_desc.Width,
        Height: source_desc.Height,
        MipLevels: 1,
        ArraySize: 1,
        Format: source_desc.Format,
        SampleDesc: source_desc.SampleDesc,
        Usage: D3D11_USAGE_STAGING,
        BindFlags: 0,
        CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
        MiscFlags: 0,
    };
    let mut staging = None;
    unsafe { device.CreateTexture2D(&staging_desc, None, Some(&mut staging)) }
        .map_err(|error| format!("failed to create staging texture: {error}"))?;
    let staging = staging.ok_or_else(|| "D3D11 returned no staging texture".to_string())?;

    unsafe { context.CopyResource(&staging, &texture) };
    let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
    unsafe { context.Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped)) }
        .map_err(|error| format!("failed to map captured texture: {error}"))?;
    let result = unsafe { copy_mapped_bgra(&mapped, width, height) };
    unsafe { context.Unmap(&staging, 0) };
    result
}

unsafe fn copy_mapped_bgra(
    mapped: &D3D11_MAPPED_SUBRESOURCE,
    width: u32,
    height: u32,
) -> Result<CapturedRgbaFrame, String> {
    if mapped.pData.is_null() {
        return Err("mapped capture texture has no data".to_string());
    }
    let row_bytes = width as usize * 4;
    let row_pitch = mapped.RowPitch as usize;
    if row_pitch < row_bytes {
        return Err(format!(
            "captured texture row pitch {row_pitch} is smaller than {row_bytes}"
        ));
    }

    let mut pixels = vec![0_u8; row_bytes * height as usize];
    let source = mapped.pData as *const u8;
    for y in 0..height as usize {
        let source_row =
            unsafe { std::slice::from_raw_parts(source.add(y * row_pitch), row_bytes) };
        let output_row = &mut pixels[y * row_bytes..(y + 1) * row_bytes];
        for (source_pixel, output_pixel) in source_row
            .chunks_exact(4)
            .zip(output_row.chunks_exact_mut(4))
        {
            output_pixel[0] = source_pixel[2];
            output_pixel[1] = source_pixel[1];
            output_pixel[2] = source_pixel[0];
            output_pixel[3] = 255;
        }
    }
    Ok(CapturedRgbaFrame {
        pixels,
        width,
        height,
    })
}
