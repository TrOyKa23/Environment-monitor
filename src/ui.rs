use core::fmt::Write as _;
use core::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use embedded_graphics::Pixel;
use embedded_graphics::geometry::Angle;
use embedded_graphics::mono_font::MonoTextStyle;
use embedded_graphics::mono_font::ascii::{FONT_6X10, FONT_10X20};
use embedded_graphics::pixelcolor::Rgb565;
use embedded_graphics::prelude::*;
use embedded_graphics::primitives::{Arc, Circle, Line, Polyline, PrimitiveStyle, Rectangle};
use embedded_graphics::text::Text;
use heapless::String;

use crate::daily_history::BUCKETS;
use crate::rtc;

const SCREEN_WIDTH: i32 = 320;
const HEADER_HEIGHT: i32 = 24;
const ROW_HEIGHT: i32 = (240 - HEADER_HEIGHT) / 2;
const HISTORY_LEN: usize = 60;

const TEMP_STEP: f32 = 0.1;
const PRESSURE_STEP: f32 = 10.0;
const STEPS_VISIBLE: f32 = 8.0;

static GRAPH_OPEN: AtomicBool = AtomicBool::new(false);

pub fn toggle_graph() {
    GRAPH_OPEN.fetch_xor(true, Ordering::Relaxed);
}

pub fn graph_is_open() -> bool {
    GRAPH_OPEN.load(Ordering::Relaxed)
}

const WIFI_DISCONNECTED: u8 = 0;
const WIFI_CONNECTING: u8 = 1;
const WIFI_CONNECTED: u8 = 2;
static WIFI_STATUS: AtomicU8 = AtomicU8::new(WIFI_DISCONNECTED);

pub fn set_wifi_connecting() {
    WIFI_STATUS.store(WIFI_CONNECTING, Ordering::Relaxed);
}

pub fn set_wifi_connected() {
    WIFI_STATUS.store(WIFI_CONNECTED, Ordering::Relaxed);
}

fn wifi_status() -> u8 {
    WIFI_STATUS.load(Ordering::Relaxed)
}

const WIFI_ICON_TOP_LEFT: Point = Point::new(292, 2);

struct History {
    buf: [f32; HISTORY_LEN],
    len: usize,
    head: usize,
}

impl History {
    const fn new() -> Self {
        Self {
            buf: [0.0; HISTORY_LEN],
            len: 0,
            head: 0,
        }
    }

    fn push(&mut self, value: f32) {
        self.buf[self.head] = value;
        self.head = (self.head + 1) % HISTORY_LEN;
        if self.len < HISTORY_LEN {
            self.len += 1;
        }
    }

    ///  val from old to new
    fn iter_in_order(&self) -> impl Iterator<Item = f32> + '_ {
        let start = if self.len < HISTORY_LEN { 0 } else { self.head };
        (0..self.len).map(move |i| self.buf[(start + i) % HISTORY_LEN])
    }
}

struct Scaled<'a, D> {
    target: &'a mut D,
    scale_x: i32,
    scale_y: i32,
    offset: Point,
}

impl<'a, D> Scaled<'a, D> {
    fn new(target: &'a mut D, scale_x: i32, scale_y: i32, offset: Point) -> Self {
        Self {
            target,
            scale_x,
            scale_y,
            offset,
        }
    }
}

impl<'a, D: DrawTarget> OriginDimensions for Scaled<'a, D> {
    fn size(&self) -> Size {
        Size::new(4096, 4096)
    }
}

impl<'a, D: DrawTarget> DrawTarget for Scaled<'a, D> {
    type Color = D::Color;
    type Error = D::Error;

    fn draw_iter<I>(&mut self, pixels: I) -> Result<(), Self::Error>
    where
        I: IntoIterator<Item = Pixel<Self::Color>>,
    {
        for Pixel(pos, color) in pixels {
            let x = self.offset.x + pos.x * self.scale_x;
            let y = self.offset.y + pos.y * self.scale_y;
            let block = Rectangle::new(
                Point::new(x, y),
                Size::new(self.scale_x as u32, self.scale_y as u32),
            );
            self.target.fill_solid(&block, color)?;
        }
        Ok(())
    }
}

fn draw_big_number<D>(display: &mut D, text: &str, position: Point, scale_x: i32, scale_y: i32)
where
    D: DrawTarget<Color = Rgb565>,
{
    let text_style = MonoTextStyle::new(&FONT_10X20, Rgb565::WHITE);
    let mut scaled = Scaled::new(display, scale_x, scale_y, position);
    let _ = Text::new(text, Point::zero(), text_style).draw(&mut scaled);
}

// history graph draw
fn draw_sparkline<D>(display: &mut D, outer_rect: Rectangle, history: &History, step: f32)
where
    D: DrawTarget<Color = Rgb565>,
{
    // frame
    let _ = outer_rect
        .into_styled(PrimitiveStyle::with_stroke(Rgb565::new(10, 20, 10), 1))
        .draw(display);

    let n = history.len;
    if n < 2 {
        return;
    }

    // inner rect - inset by 2px from outer rect, to leave space for frame
    let inner = Rectangle::new(
        outer_rect.top_left + Point::new(2, 2),
        Size::new(
            outer_rect.size.width.saturating_sub(4),
            outer_rect.size.height.saturating_sub(4),
        ),
    );

    let sum: f32 = history.iter_in_order().sum();
    let center = sum / n as f32;

    let w = inner.size.width as i32;
    let h = inner.size.height as i32;
    let mid_y = inner.top_left.y + h / 2;

    // fixed scale for 1 step on y axis
    let pixels_per_unit = h as f32 / (STEPS_VISIBLE * step);

    let mut points: heapless::Vec<Point, HISTORY_LEN> = heapless::Vec::new();
    for (i, v) in history.iter_in_order().enumerate() {
        let x = inner.top_left.x + (i as i32 * w) / (n as i32 - 1).max(1);
        let delta = v - center;
        let y = mid_y - (delta * pixels_per_unit) as i32;
        let y = y.clamp(inner.top_left.y, inner.top_left.y + h);
        let _ = points.push(Point::new(x, y));
    }

    let _ = Polyline::new(&points)
        .into_styled(PrimitiveStyle::with_stroke(Rgb565::WHITE, 1))
        .draw(display);
}

enum IconKind {
    Thermometer,
    Pressure,
}

fn draw_thermometer_icon<D>(display: &mut D, top_left: Point)
where
    D: DrawTarget<Color = Rgb565>,
{
    let stroke = PrimitiveStyle::with_stroke(Rgb565::WHITE, 3);
    let fill = PrimitiveStyle::with_fill(Rgb565::WHITE);

    let _ = Line::new(top_left + Point::new(8, 0), top_left + Point::new(8, 26))
        .into_styled(stroke)
        .draw(display);
    let _ = Circle::new(top_left + Point::new(2, 24), 12)
        .into_styled(fill)
        .draw(display);
}

fn draw_pressure_icon<D>(display: &mut D, top_left: Point)
where
    D: DrawTarget<Color = Rgb565>,
{
    let stroke = PrimitiveStyle::with_stroke(Rgb565::WHITE, 3);
    for (i, w) in [24, 18, 12].iter().enumerate() {
        let y = i as i32 * 10;
        let x_offset = (24 - w) / 2;
        let _ = Line::new(
            top_left + Point::new(x_offset, y),
            top_left + Point::new(x_offset + w, y),
        )
        .into_styled(stroke)
        .draw(display);
    }
}

fn draw_sync_icon<D>(display: &mut D, center: Point)
where
    D: DrawTarget<Color = Rgb565>,
{
    let stroke = PrimitiveStyle::with_stroke(Rgb565::WHITE, 2);
    let _ = Arc::new(
        center,
        14,
        Angle::from_degrees(20.0),
        Angle::from_degrees(280.0),
    )
    .into_styled(stroke)
    .draw(display);
}

// wifi icon
fn draw_wifi_icon_shape<D>(display: &mut D, top_left: Point, color: Rgb565)
where
    D: DrawTarget<Color = Rgb565>,
{
    let stroke = PrimitiveStyle::with_stroke(color, 2);
    for (i, d) in [6i32, 12, 18].iter().enumerate() {
        let inset = (18 - d) / 2;
        let _ = Arc::new(
            top_left + Point::new(inset, 18 - d - 2 * i as i32),
            *d as u32,
            Angle::from_degrees(215.0),
            Angle::from_degrees(110.0),
        )
        .into_styled(stroke)
        .draw(display);
    }
    let _ = Circle::new(top_left + Point::new(7, 16), 4)
        .into_styled(PrimitiveStyle::with_fill(color))
        .draw(display);
}

// icon if wifi not connected
fn draw_wifi_disconnected<D>(display: &mut D, top_left: Point)
where
    D: DrawTarget<Color = Rgb565>,
{
    let dim = Rgb565::new(8, 16, 8);
    draw_wifi_icon_shape(display, top_left, dim);
    let stroke = PrimitiveStyle::with_stroke(Rgb565::new(20, 8, 8), 2);
    let _ = Line::new(top_left + Point::new(0, 0), top_left + Point::new(18, 18))
        .into_styled(stroke)
        .draw(display);
    let _ = Line::new(top_left + Point::new(18, 0), top_left + Point::new(0, 18))
        .into_styled(stroke)
        .draw(display);
}

// wifi (connection) icon animation
fn draw_wifi_connecting<D>(display: &mut D, top_left: Point, frame: usize)
where
    D: DrawTarget<Color = Rgb565>,
{
    let stroke = PrimitiveStyle::with_stroke(Rgb565::new(24, 40, 0), 2);
    let start_deg = (frame % 8) as f32 * 45.0;
    let _ = Arc::new(
        top_left + Point::new(9, 9),
        18,
        Angle::from_degrees(start_deg),
        Angle::from_degrees(90.0),
    )
    .into_styled(stroke)
    .draw(display);
}

// wifi (connected) icon
fn draw_wifi_connected<D>(display: &mut D, top_left: Point)
where
    D: DrawTarget<Color = Rgb565>,
{
    draw_wifi_icon_shape(display, top_left, Rgb565::new(0, 55, 5));
}

fn draw_wifi_icon<D>(display: &mut D, top_left: Point, frame: usize)
where
    D: DrawTarget<Color = Rgb565>,
{
    match wifi_status() {
        WIFI_CONNECTED => draw_wifi_connected(display, top_left),
        WIFI_CONNECTING => draw_wifi_connecting(display, top_left, frame),
        _ => draw_wifi_disconnected(display, top_left),
    }
}

// higher refreshrate for wifi icon animation
pub fn redraw_wifi_icon<D>(display: &mut D, frame: usize)
where
    D: DrawTarget<Color = Rgb565>,
{
    if graph_is_open() {
        return;
    }
    let erase_rect = Rectangle::new(Point::new(286, 0), Size::new(34, HEADER_HEIGHT as u32));
    let _ = erase_rect
        .into_styled(PrimitiveStyle::with_fill(Rgb565::BLACK))
        .draw(display);
    draw_wifi_icon(display, WIFI_ICON_TOP_LEFT, frame);
}

fn draw_bt_off_icon<D>(display: &mut D, top_left: Point)
where
    D: DrawTarget<Color = Rgb565>,
{
    let stroke = PrimitiveStyle::with_stroke(Rgb565::WHITE, 2);
    let _ = Circle::new(top_left, 16).into_styled(stroke).draw(display);
    let _ = Line::new(top_left + Point::new(2, 2), top_left + Point::new(14, 14))
        .into_styled(stroke)
        .draw(display);
}

fn draw_header<D>(display: &mut D, uptime_secs: u64, anim_frame: usize)
where
    D: DrawTarget<Color = Rgb565>,
{
    let text_style = MonoTextStyle::new(&FONT_6X10, Rgb565::WHITE);

    // NTP sync date, time else placeholder
    let mut date_str: String<10> = String::new();
    match rtc::now_datetime() {
        Some(dt) => {
            let _ = write!(
                date_str,
                "{:02}:{:02}:{:02}",
                dt.day,
                dt.month,
                dt.year % 100
            );
        }
        None => {
            let _ = write!(date_str, "--:--:--");
        }
    }
    let _ = Text::new(&date_str, Point::new(4, 10), text_style).draw(display);

    // NTP sync time
    let (h, m) = match rtc::now_datetime() {
        Some(dt) => (dt.hour as u64, dt.minute as u64),
        None => ((uptime_secs / 3600) % 100, (uptime_secs % 3600) / 60),
    };
    let mut time_str: String<8> = String::new();
    let _ = write!(time_str, "{:02}:{:02}", h, m);
    let _ = Text::new(&time_str, Point::new(140, 10), text_style).draw(display);

    // decorations (hoping to make them functional (wifi data sync transfer and wifi connectivity))
    draw_sync_icon(display, Point::new(236, 8));
    draw_bt_off_icon(display, Point::new(262, 2));
    draw_wifi_icon(display, Point::new(292, 2), anim_frame);

    let _ = Line::new(
        Point::new(0, HEADER_HEIGHT - 2),
        Point::new(SCREEN_WIDTH, HEADER_HEIGHT - 2),
    )
    .into_styled(PrimitiveStyle::with_stroke(Rgb565::new(8, 16, 8), 1))
    .draw(display);
}

fn draw_row<D>(
    display: &mut D,
    row_top: i32,
    icon: IconKind,
    value: f32,
    history: &History,
    step: f32,
) where
    D: DrawTarget<Color = Rgb565>,
{
    let number_y = row_top + (ROW_HEIGHT - 40) / 2 + 50;

    match icon {
        IconKind::Thermometer => draw_thermometer_icon(display, Point::new(6, row_top + 24)),
        IconKind::Pressure => draw_pressure_icon(display, Point::new(6, row_top + 34)),
    }

    let mut text: String<8> = String::new();
    let _ = write!(text, "{:.1}", value);
    draw_big_number(display, &text, Point::new(54, number_y), 2, 5);

    let graph_rect = Rectangle::new(
        Point::new(198, row_top + 8),
        Size::new(116, (ROW_HEIGHT - 16) as u32),
    );
    draw_sparkline(display, graph_rect, history, step);
}

// additional graph for last 24h temperature
fn draw_daily_graph<D>(display: &mut D, points: &[Option<f32>; BUCKETS])
where
    D: DrawTarget<Color = Rgb565>,
{
    let _ = display.clear(Rgb565::BLACK);

    let text_style = MonoTextStyle::new(&FONT_6X10, Rgb565::WHITE);
    let _ = Text::new(
        "Temp - last 24h (tap button to close)",
        Point::new(6, 12),
        text_style,
    )
    .draw(display);

    let margin_left = 34;
    let margin_right = 8;
    let margin_top = 22;
    let margin_bottom = 20;
    let chart_rect = Rectangle::new(
        Point::new(margin_left, margin_top),
        Size::new(
            (SCREEN_WIDTH - margin_left - margin_right) as u32,
            (240 - margin_top - margin_bottom) as u32,
        ),
    );
    let _ = chart_rect
        .into_styled(PrimitiveStyle::with_stroke(Rgb565::new(10, 20, 10), 1))
        .draw(display);

    let known: heapless::Vec<(usize, f32), BUCKETS> = points
        .iter()
        .enumerate()
        .filter_map(|(i, v)| v.map(|val| (i, val)))
        .collect();

    if known.len() < 2 {
        let _ = Text::new(
            "Not enough data yet",
            Point::new(margin_left + 10, margin_top + 40),
            text_style,
        )
        .draw(display);
        let _ = Text::new(
            "(keeps a point every 30 min)",
            Point::new(margin_left + 10, margin_top + 52),
            text_style,
        )
        .draw(display);
        return;
    }

    let min = known.iter().map(|(_, v)| *v).fold(f32::INFINITY, f32::min);
    let max = known
        .iter()
        .map(|(_, v)| *v)
        .fold(f32::NEG_INFINITY, f32::max);
    // 0 devide protection + some margin for better visual
    let span = (max - min).max(0.5) * 1.2;
    let mid = (max + min) / 2.0;

    let w = chart_rect.size.width as i32;
    let h = chart_rect.size.height as i32;

    let mut poly_points: heapless::Vec<Point, BUCKETS> = heapless::Vec::new();
    for (i, v) in known.iter() {
        let x = chart_rect.top_left.x + (*i as i32 * w) / (BUCKETS as i32 - 1).max(1);
        let delta = *v - mid;
        let y = chart_rect.top_left.y + h / 2 - ((delta / span) * h as f32) as i32;
        let y = y.clamp(chart_rect.top_left.y, chart_rect.top_left.y + h);
        let _ = poly_points.push(Point::new(x, y));
    }

    let _ = Polyline::new(&poly_points)
        .into_styled(PrimitiveStyle::with_stroke(Rgb565::WHITE, 1))
        .draw(display);

    // max and min labels
    let mut max_str: String<8> = String::new();
    let _ = write!(max_str, "{:.1}", max);
    let _ = Text::new(&max_str, Point::new(2, margin_top + 8), text_style).draw(display);

    let mut min_str: String<8> = String::new();
    let _ = write!(min_str, "{:.1}", min);
    let _ = Text::new(&min_str, Point::new(2, margin_top + h - 2), text_style).draw(display);

    let _ = Text::new("-24h", Point::new(margin_left, 236), text_style).draw(display);
    let _ = Text::new("now", Point::new(SCREEN_WIDTH - 28, 236), text_style).draw(display);
}

pub struct Dashboard {
    temp_history: History,
    pressure_history: History,
}

impl Dashboard {
    pub fn new() -> Self {
        Self {
            temp_history: History::new(),
            pressure_history: History::new(),
        }
    }

    //full lcd refresh with new values
    pub fn update<D>(
        &mut self,
        display: &mut D,
        uptime_secs: u64,
        temp_c: f32,
        pressure_hpa: f32,
        daily_temps: &[Option<f32>; BUCKETS],
        anim_frame: usize,
    ) where
        D: DrawTarget<Color = Rgb565>,
    {
        self.temp_history.push(temp_c);
        self.pressure_history.push(pressure_hpa);

        if graph_is_open() {
            draw_daily_graph(display, daily_temps);
            return;
        }

        let _ = display.clear(Rgb565::BLACK);

        draw_header(display, uptime_secs, anim_frame);

        draw_row(
            display,
            HEADER_HEIGHT,
            IconKind::Thermometer,
            temp_c,
            &self.temp_history,
            TEMP_STEP,
        );
        draw_row(
            display,
            HEADER_HEIGHT + ROW_HEIGHT,
            IconKind::Pressure,
            pressure_hpa,
            &self.pressure_history,
            PRESSURE_STEP,
        );
    }
}
