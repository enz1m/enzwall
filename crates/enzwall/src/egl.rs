#![allow(
    unsafe_code,
    reason = "EGL context creation and GL symbol loading require native handles."
)]
use khronos_egl as egl;
use std::ffi::c_void;

pub(crate) type Egl = egl::Instance<egl::Static>;

const PLATFORM_WAYLAND: egl::Enum = 0x31D8;
const PLATFORM_SURFACELESS: egl::Enum = 0x31DD;

pub(crate) struct Context {
    pub egl: Egl,
    pub display: egl::Display,
    pub config: egl::Config,
    pub context: egl::Context,
    swap_with_damage: Option<SwapWithDamage>,
}

type SwapWithDamage =
    extern "system" fn(egl::EGLDisplay, egl::EGLSurface, *const i32, i32) -> egl::Boolean;

impl Context {
    fn create(platform: egl::Enum, native: *mut c_void, window: bool) -> Result<Self, String> {
        let egl = egl::Instance::new(egl::Static);
        // SAFETY: callers supply a live Wayland display or NULL for the surfaceless platform.
        let display = unsafe { egl.get_platform_display(platform, native, &[egl::ATTRIB_NONE]) }
            .map_err(|e| format!("EGL display: {e}"))?;
        egl.initialize(display)
            .map_err(|e| format!("EGL initialise: {e}"))?;
        egl.bind_api(egl::OPENGL_ES_API)
            .map_err(|e| format!("EGL API: {e}"))?;
        let attributes = [
            egl::SURFACE_TYPE,
            if window {
                egl::WINDOW_BIT
            } else {
                egl::PBUFFER_BIT
            },
            egl::RENDERABLE_TYPE,
            egl::OPENGL_ES3_BIT,
            egl::RED_SIZE,
            8,
            egl::GREEN_SIZE,
            8,
            egl::BLUE_SIZE,
            8,
            egl::ALPHA_SIZE,
            0,
            egl::NONE,
        ];
        let config = egl
            .choose_first_config(display, &attributes)
            .map_err(|e| format!("EGL config: {e}"))?
            .ok_or("No OpenGL ES 3 EGL configuration")?;
        let context = egl
            .create_context(
                display,
                config,
                None,
                &[egl::CONTEXT_MAJOR_VERSION, 3, egl::NONE],
            )
            .map_err(|e| format!("EGL context: {e}"))?;
        let extensions = egl
            .query_string(Some(display), egl::EXTENSIONS)
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let swap_with_damage = [
            (
                "EGL_KHR_swap_buffers_with_damage",
                "eglSwapBuffersWithDamageKHR",
            ),
            (
                "EGL_EXT_swap_buffers_with_damage",
                "eglSwapBuffersWithDamageEXT",
            ),
        ]
        .into_iter()
        .filter(|(extension, _)| extensions.split(' ').any(|e| e == *extension))
        .find_map(|(_, name)| egl.get_proc_address(name))
        .map(|f| {
            // SAFETY: each advertised extension name resolves the specified EGL swap-with-damage ABI.
            unsafe { std::mem::transmute::<extern "system" fn(), SwapWithDamage>(f) }
        });
        Ok(Self {
            egl,
            display,
            config,
            context,
            swap_with_damage,
        })
    }

    pub(crate) fn wayland(display: *mut c_void) -> Result<Self, String> {
        Self::create(PLATFORM_WAYLAND, display, true)
    }

    pub(crate) fn surfaceless() -> Result<Self, String> {
        Self::create(PLATFORM_SURFACELESS, std::ptr::null_mut(), false)
    }

    pub(crate) fn glow(&self) -> glow::Context {
        // SAFETY: callers made this EGL context current and retain it while the GL loader is used.
        unsafe {
            glow::Context::from_loader_function(|name| {
                self.egl
                    .get_proc_address(name)
                    .map_or(std::ptr::null(), |f| f as *const c_void)
            })
        }
    }

    pub(crate) fn make_current(&self, surface: Option<egl::Surface>) -> Result<(), String> {
        self.egl
            .make_current(self.display, surface, surface, Some(self.context))
            .map_err(|e| format!("EGL make current: {e}"))
    }

    /// Presents the back buffer, telling the compositor only `damage`
    /// (buffer pixels, origin top-left) changed when the driver supports it.
    pub(crate) fn swap(
        &self,
        surface: egl::Surface,
        damage: Option<&[[i32; 4]]>,
        height: i32,
    ) -> Result<(), String> {
        if let (Some(swap), Some(rects)) = (self.swap_with_damage, damage) {
            let flipped: Vec<i32> = rects
                .iter()
                .flat_map(|r| [r[0], height - r[1] - r[3], r[2], r[3]])
                .collect();
            if swap(
                self.display.as_ptr(),
                surface.as_ptr(),
                flipped.as_ptr(),
                rects.len() as i32,
            ) == egl::TRUE
            {
                return Ok(());
            }
        }
        self.egl
            .swap_buffers(self.display, surface)
            .map_err(|e| format!("EGL swap: {e}"))
    }

    pub(crate) fn has_partial_damage(&self) -> bool {
        self.swap_with_damage.is_some()
    }
}

impl Drop for Context {
    fn drop(&mut self) {
        let _ = self.egl.make_current(self.display, None, None, None);
        let _ = self.egl.destroy_context(self.display, self.context);
        let _ = self.egl.terminate(self.display);
    }
}
