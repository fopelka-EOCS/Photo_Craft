//! #6419: the compositor's pipelines are built on several threads. A pipeline the device can't
//! build is still an `Err` from `try_new_with_format`, not a panic from wgpu's default error
//! handler (error scopes are per thread). Skips when no GPU adapter exists.

use photocraft_gpu::Compositor;

#[test]
fn a_pipeline_built_on_another_thread_fails_as_an_error() {
    let instance = wgpu::Instance::default();
    let Ok(adapter) = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())) else {
        eprintln!("skipping: no adapter");
        return;
    };
    let Ok((device, _queue)) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())) else { return };
    // Never a render target, so every accumulation pipeline fails validation.
    let Err(e) = Compositor::try_new_with_format(&device, wgpu::TextureFormat::Rgb9e5Ufloat) else {
        panic!("a compositor accumulating into Rgb9e5Ufloat");
    };
    assert!(e.0.contains("couldn't build the compositor's pipelines"), "{e:?}");
    // The device is still usable for a compositor it can build.
    assert!(Compositor::try_new_with_format(&device, Compositor::preferred_acc_format(&adapter)).is_ok());
}
