//! Small widgets: a slider that does not fight the updates coming back from the lights, a
//! colour dot, and a hue/saturation wheel.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::{Duration, Instant};

use gtk::prelude::*;
use gtk::{cairo, gdk};

use crate::lights::hsv_to_rgb;

type Cb<T> = Rc<RefCell<Option<Box<dyn Fn(T)>>>>;

/// A slider with an icon in front and a value on the right.
#[derive(Clone)]
pub struct SliderRow {
    pub root: gtk::Box,
    pub scale: gtk::Scale,
    value: gtk::Label,
    format: Rc<dyn Fn(f64) -> String>,
    guard: Rc<Cell<bool>>,
    last_user: Rc<Cell<Option<Instant>>>,
    cb: Cb<f64>,
}

impl SliderRow {
    pub fn new(icon: &str, min: f64, max: f64, step: f64, format: impl Fn(f64) -> String + 'static) -> SliderRow {
        let root = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        root.add_css_class("lm-row");
        let img = gtk::Image::from_icon_name(icon);
        img.add_css_class("lm-row-icon");
        let scale = gtk::Scale::with_range(gtk::Orientation::Horizontal, min, max, step);
        scale.set_hexpand(true);
        scale.set_draw_value(false);
        scale.add_css_class("lm-slider");
        let value = gtk::Label::new(None);
        value.add_css_class("lm-value");
        value.set_width_chars(6);
        value.set_xalign(1.0);
        root.append(&img);
        root.append(&scale);
        root.append(&value);
        let format: Rc<dyn Fn(f64) -> String> = Rc::new(format);
        let row = SliderRow { root, scale: scale.clone(), value, format, guard: Rc::new(Cell::new(false)), last_user: Rc::new(Cell::new(None)), cb: Rc::new(RefCell::new(None)) };
        let r2 = row.clone();
        scale.connect_value_changed(move |s| {
            let v = s.value();
            r2.value.set_text(&(r2.format)(v));
            if r2.guard.get() {
                return;
            }
            r2.last_user.set(Some(Instant::now()));
            if let Some(cb) = r2.cb.borrow().as_ref() {
                cb(v);
            }
        });
        row.value.set_text(&(row.format)(scale.value()));
        row
    }

    pub fn connect_changed(&self, f: impl Fn(f64) + 'static) {
        *self.cb.borrow_mut() = Some(Box::new(f));
    }

    /// Set from the outside. Ignored for a moment after the user touched it, so that an
    /// older answer from the light does not pull the slider back under the hand.
    pub fn set_quiet(&self, v: f64) {
        if self.touched_recently() {
            return;
        }
        self.guard.set(true);
        self.scale.set_value(v);
        self.guard.set(false);
        self.value.set_text(&(self.format)(v));
    }

    pub fn touched_recently(&self) -> bool {
        self.last_user.get().map(|t| t.elapsed() < Duration::from_millis(1500)).unwrap_or(false)
    }

    pub fn set_range(&self, min: f64, max: f64) {
        self.guard.set(true);
        self.scale.set_range(min, max);
        self.guard.set(false);
    }
}

/// A round dot in the colour the light shows; dim when it is off.
#[derive(Clone)]
pub struct Dot {
    pub area: gtk::DrawingArea,
    color: Rc<Cell<((u8, u8, u8), bool)>>,
}

impl Dot {
    pub fn new(size: i32) -> Dot {
        let area = gtk::DrawingArea::new();
        area.set_content_width(size);
        area.set_content_height(size);
        area.set_valign(gtk::Align::Center);
        let color: Rc<Cell<((u8, u8, u8), bool)>> = Rc::new(Cell::new(((255, 255, 255), false)));
        let c = color.clone();
        area.set_draw_func(move |_, cr, w, h| {
            let ((r, g, b), on) = c.get();
            let rad = (w.min(h) as f64) / 2.0 - 1.0;
            let (cx, cy) = (w as f64 / 2.0, h as f64 / 2.0);
            if on {
                // soft glow, then the dot
                let glow = cairo::RadialGradient::new(cx, cy, rad * 0.4, cx, cy, rad);
                glow.add_color_stop_rgba(0.0, r as f64 / 255.0, g as f64 / 255.0, b as f64 / 255.0, 0.55);
                glow.add_color_stop_rgba(1.0, r as f64 / 255.0, g as f64 / 255.0, b as f64 / 255.0, 0.0);
                cr.set_source(&glow).ok();
                cr.arc(cx, cy, rad, 0.0, std::f64::consts::TAU);
                cr.fill().ok();
                cr.set_source_rgb(r as f64 / 255.0, g as f64 / 255.0, b as f64 / 255.0);
                cr.arc(cx, cy, rad * 0.62, 0.0, std::f64::consts::TAU);
                cr.fill().ok();
            } else {
                cr.set_source_rgba(0.5, 0.5, 0.55, 0.35);
                cr.set_line_width(2.0);
                cr.arc(cx, cy, rad * 0.55, 0.0, std::f64::consts::TAU);
                cr.stroke().ok();
            }
        });
        Dot { area, color }
    }

    pub fn set(&self, rgb: (u8, u8, u8), on: bool) {
        if self.color.get() != (rgb, on) {
            self.color.set((rgb, on));
            self.area.queue_draw();
        }
    }
}

/// Hue around, saturation outwards; drag anywhere on it.
#[derive(Clone)]
pub struct Wheel {
    pub area: gtk::DrawingArea,
    hs: Rc<Cell<(f64, f64)>>,
    cache: Rc<RefCell<Option<(i32, cairo::ImageSurface)>>>,
    cb: Cb<(f64, f64)>,
    last_user: Rc<Cell<Option<Instant>>>,
}

impl Wheel {
    pub fn new(size: i32) -> Wheel {
        let area = gtk::DrawingArea::new();
        area.set_content_width(size);
        area.set_content_height(size);
        area.add_css_class("lm-wheel");
        area.set_cursor_from_name(Some("crosshair"));
        let w = Wheel { area: area.clone(), hs: Rc::new(Cell::new((0.0, 0.0))), cache: Rc::new(RefCell::new(None)), cb: Rc::new(RefCell::new(None)), last_user: Rc::new(Cell::new(None)) };
        {
            let w2 = w.clone();
            area.set_draw_func(move |a, cr, width, height| w2.draw(a, cr, width, height));
        }
        let drag = gtk::GestureDrag::new();
        {
            let w2 = w.clone();
            drag.connect_drag_begin(move |_, x, y| w2.pick(x, y));
        }
        {
            let w2 = w.clone();
            drag.connect_drag_update(move |g, dx, dy| {
                if let Some((x, y)) = g.start_point() {
                    w2.pick(x + dx, y + dy);
                }
            });
        }
        area.add_controller(drag);
        w
    }

    fn pick(&self, x: f64, y: f64) {
        let (wd, ht) = (self.area.width() as f64, self.area.height() as f64);
        let r = wd.min(ht) / 2.0 - 4.0;
        let (dx, dy) = (x - wd / 2.0, y - ht / 2.0);
        let hue = (dy.atan2(dx).to_degrees() + 360.0) % 360.0;
        let sat = ((dx * dx + dy * dy).sqrt() / r).clamp(0.0, 1.0);
        self.hs.set((hue, sat));
        self.last_user.set(Some(Instant::now()));
        self.area.queue_draw();
        if let Some(cb) = self.cb.borrow().as_ref() {
            cb((hue, sat));
        }
    }

    pub fn connect_picked(&self, f: impl Fn((f64, f64)) + 'static) {
        *self.cb.borrow_mut() = Some(Box::new(f));
    }

    pub fn set_quiet(&self, hue: f64, sat: f64) {
        if self.last_user.get().map(|t| t.elapsed() < Duration::from_millis(1500)).unwrap_or(false) {
            return;
        }
        if self.hs.get() != (hue, sat) {
            self.hs.set((hue, sat));
            self.area.queue_draw();
        }
    }

    fn surface(&self, px: i32) -> Option<cairo::ImageSurface> {
        if let Some((s, surf)) = self.cache.borrow().as_ref() {
            if *s == px {
                return Some(surf.clone());
            }
        }
        let mut surf = cairo::ImageSurface::create(cairo::Format::ARgb32, px, px).ok()?;
        let stride = surf.stride() as usize;
        {
            let mut data = surf.data().ok()?;
            let c = px as f64 / 2.0;
            let r = c - 1.0;
            for y in 0..px {
                for x in 0..px {
                    let (dx, dy) = (x as f64 + 0.5 - c, y as f64 + 0.5 - c);
                    let d = (dx * dx + dy * dy).sqrt();
                    // one pixel of antialiasing at the rim
                    let a = (r - d + 0.5).clamp(0.0, 1.0);
                    if a <= 0.0 {
                        continue;
                    }
                    let hue = (dy.atan2(dx).to_degrees() + 360.0) % 360.0;
                    let (cr, cg, cb) = hsv_to_rgb(hue, (d / r).min(1.0), 1.0);
                    let i = y as usize * stride + x as usize * 4;
                    // premultiplied BGRA (little endian ARGB32)
                    data[i] = (cb as f64 * a) as u8;
                    data[i + 1] = (cg as f64 * a) as u8;
                    data[i + 2] = (cr as f64 * a) as u8;
                    data[i + 3] = (a * 255.0) as u8;
                }
            }
        }
        surf.mark_dirty();
        *self.cache.borrow_mut() = Some((px, surf.clone()));
        Some(surf)
    }

    fn draw(&self, a: &gtk::DrawingArea, cr: &cairo::Context, width: i32, height: i32) {
        let size = width.min(height) - 8;
        if size <= 0 {
            return;
        }
        let scale = a.scale_factor().max(1);
        let Some(surf) = self.surface(size * scale) else { return };
        let (ox, oy) = ((width - size) as f64 / 2.0, (height - size) as f64 / 2.0);
        cr.save().ok();
        cr.translate(ox, oy);
        cr.scale(1.0 / scale as f64, 1.0 / scale as f64);
        cr.set_source_surface(&surf, 0.0, 0.0).ok();
        cr.paint().ok();
        cr.restore().ok();
        // the marker
        let (h, s) = self.hs.get();
        let r = size as f64 / 2.0;
        let (cx, cy) = (width as f64 / 2.0 + h.to_radians().cos() * s * r, height as f64 / 2.0 + h.to_radians().sin() * s * r);
        let (mr, mg, mb) = hsv_to_rgb(h, s, 1.0);
        cr.set_source_rgba(0.0, 0.0, 0.0, 0.35);
        cr.arc(cx, cy + 1.0, 9.0, 0.0, std::f64::consts::TAU);
        cr.fill().ok();
        cr.set_source_rgb(1.0, 1.0, 1.0);
        cr.arc(cx, cy, 9.0, 0.0, std::f64::consts::TAU);
        cr.fill().ok();
        cr.set_source_rgb(mr as f64 / 255.0, mg as f64 / 255.0, mb as f64 / 255.0);
        cr.arc(cx, cy, 6.5, 0.0, std::f64::consts::TAU);
        cr.fill().ok();
    }
}

pub fn rgba(rgb: u32) -> gdk::RGBA {
    gdk::RGBA::new(((rgb >> 16) & 255) as f32 / 255.0, ((rgb >> 8) & 255) as f32 / 255.0, (rgb & 255) as f32 / 255.0, 1.0)
}

pub fn icon_button(icon: &str, tooltip: &str) -> gtk::Button {
    let b = gtk::Button::from_icon_name(icon);
    b.add_css_class("flat");
    b.add_css_class("circular");
    b.set_tooltip_text(Some(tooltip));
    b.set_valign(gtk::Align::Center);
    b
}

pub fn section_label(text: &str) -> gtk::Label {
    let l = gtk::Label::new(Some(&text.to_uppercase()));
    l.set_xalign(0.0);
    l.add_css_class("lm-section");
    l
}
