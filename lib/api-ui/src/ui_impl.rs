use std::{cell::RefCell, time::Duration};

use api::{read_param_from_host, upload_return_val, GRAPHICS, GRAPHICS_BACKEND};

use graphics_types::types::WindowProps;
use ui_base::{
    types::{RawInputWrapper, RawOutputWrapper, UiFonts, UiRenderPipe},
    ui::UiContainer,
    ui_render::render_ui,
};
use ui_traits::traits::UiPageInterface;

extern "Rust" {
    /// returns an instance of the game state and the game tick speed
    fn mod_ui_new() -> Box<dyn UiPageInterface<()>>;
}

type U = ();

static mut API_UI: once_cell::unsync::Lazy<RefCell<UiContainer>> =
    once_cell::unsync::Lazy::new(|| RefCell::new(UiContainer::new(&Default::default())));

static mut API_UI_USER: once_cell::unsync::Lazy<RefCell<Box<dyn UiPageInterface<U>>>> =
    once_cell::unsync::Lazy::new(|| RefCell::new(unsafe { mod_ui_new() }));

#[no_mangle]
pub fn ui_new() {
    let fonts = read_param_from_host::<UiFonts>(0);
    let ui = unsafe { API_UI.borrow_mut() };
    ui.context.egui_ctx.set_fonts(fonts.fonts.clone());
    *ui.font_definitions.borrow_mut() = Some(fonts.fonts);
}

/// returns platform output and zoom level
fn ui_run_impl(
    cur_time: Duration,
    window_props: WindowProps,
    inp: RawInputWrapper,
    zoom_level: Option<f32>,
    main_frame_only: bool,
    mut user_data: U,
) -> egui::PlatformOutput {
    if !main_frame_only || unsafe { API_UI_USER.borrow().has_blur() } {
        unsafe { API_UI.borrow_mut().zoom_level.set(zoom_level) };
        unsafe { GRAPHICS.borrow_mut().resized(window_props) };

        let (screen_rect, full_output, zoom_level) = unsafe {
            API_UI.borrow_mut().render(
                GRAPHICS.borrow().canvas_handle.window_width(),
                GRAPHICS.borrow().canvas_handle.window_height(),
                GRAPHICS.borrow().canvas_handle.window_pixels_per_point(),
                |ui, pipe, ui_state| {
                    if main_frame_only {
                        API_UI_USER
                            .borrow_mut()
                            .render_main_frame(ui, pipe, ui_state);
                    } else {
                        API_UI_USER.borrow_mut().render(ui, pipe, ui_state);
                    }
                },
                &mut UiRenderPipe::new(cur_time, &mut user_data),
                inp.input,
                main_frame_only,
            )
        };

        let platform_output = unsafe {
            let graphics = GRAPHICS.borrow();
            render_ui(
                &mut API_UI.borrow_mut(),
                full_output,
                &screen_rect,
                zoom_level,
                &graphics.backend_handle,
                &graphics.texture_handle,
                &graphics.stream_handle,
                main_frame_only,
            )
        };

        unsafe { &mut *GRAPHICS_BACKEND }.actual_run_cmds.set(false);
        let graphics = unsafe { &mut *GRAPHICS };
        graphics
            .borrow()
            .backend_handle
            .run_backend_buffer(graphics.borrow().stream_handle.stream_data());
        unsafe { &mut *GRAPHICS_BACKEND }.actual_run_cmds.set(true);

        platform_output
    } else {
        Default::default()
    }
}

#[no_mangle]
pub fn ui_has_blur() -> u8 {
    match unsafe { API_UI_USER.borrow().has_blur() } {
        true => 1,
        false => 0,
    }
}

#[no_mangle]
pub fn ui_mount() {
    unsafe {
        API_UI_USER.borrow_mut().mount();
    }
}

#[no_mangle]
pub fn ui_unmount() {
    unsafe {
        API_UI_USER.borrow_mut().unmount();
    }
}

#[no_mangle]
pub fn ui_main_frame() {
    let cur_time = read_param_from_host::<Duration>(0);
    let window_props = read_param_from_host::<WindowProps>(1);
    let zoom_level = read_param_from_host::<Option<f32>>(2);

    ui_run_impl(
        cur_time,
        window_props,
        RawInputWrapper {
            input: Default::default(),
        },
        zoom_level,
        true,
        (),
    );
}

#[no_mangle]
pub fn ui_run() {
    let cur_time = read_param_from_host::<Duration>(0);
    let window_props = read_param_from_host::<WindowProps>(1);
    let inp = read_param_from_host::<RawInputWrapper>(2);
    let zoom_level = read_param_from_host::<Option<f32>>(3);

    let output = ui_run_impl(cur_time, window_props, inp, zoom_level, false, ());
    upload_return_val(RawOutputWrapper {
        output,
        zoom_level: unsafe { API_UI.borrow_mut().zoom_level.get() },
    });
}
