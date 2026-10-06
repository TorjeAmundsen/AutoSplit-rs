//! Ports of the OpenCV 4.11 routines AutoSplit's comparisons go through.
//!
//! They follow OpenCV's integer and floating point rounding step by step, so that similarity
//! values match AutoSplit for the same inputs. OpenCV is licensed under Apache-2.0, see
//! THIRD_PARTY_NOTICES.md. Only 8-bit images are supported, because that's all AutoSplit uses.

use std::ops::Range;

use crate::image::{BgraImage, BgraView};

/// Working memory for the resizes, kept between frames so they don't allocate.
#[derive(Default)]
pub struct ResizeBuffers {
    x_offsets: Vec<usize>,
    column_sums: Vec<u32>,
    xtab: Vec<DecimateAlpha>,
    ytab: Vec<DecimateAlpha>,
    column_depths: Vec<usize>,
    column_table: Vec<(usize, f32)>,
    row_buf: Vec<f32>,
    row_sum: Vec<f32>,
    x_axis: LinearExactAxis,
    y_axis: LinearExactAxis,
    lines: Vec<u16>,
}

/// `cvRound` for floats and doubles: round half to even.
fn cv_round(value: f64) -> i64 {
    value.round_ties_even() as i64
}

fn saturate_u8(value: i64) -> u8 {
    value.clamp(0, 255) as u8
}

fn nearest_x_offsets(src_width: usize, dst_width: usize, ifx: f64, x_offsets: &mut Vec<usize>) {
    x_offsets.clear();
    x_offsets.extend((0..dst_width).map(|x| ((x as f64 * ifx).floor() as usize).min(src_width - 1)));
}

fn nearest_y(y: usize, ify: f64, src_height: usize) -> usize {
    ((y as f64 * ify).floor() as usize).min(src_height - 1)
}

/// `cv::resize(..., INTER_NEAREST)`.
///
/// `inv_scale_x`/`inv_scale_y` are OpenCV's `fx`/`fy`. With an explicit size OpenCV uses
/// `dst / src`; with `dsize=None` it uses the factor as given, which can differ in the last bits.
pub fn resize_nearest(
    src: BgraView,
    dst_width: usize,
    dst_height: usize,
    inv_scale_x: f64,
    inv_scale_y: f64,
    buffers: &mut ResizeBuffers,
    dst: &mut BgraImage,
) {
    dst.reshape(dst_width, dst_height);
    if dst_width == src.width() && dst_height == src.height() {
        for (y, dst_row) in dst.data_mut().chunks_exact_mut(dst_width * 4).enumerate() {
            dst_row.copy_from_slice(src.row(y));
        }
        return;
    }

    let ify = 1.0 / inv_scale_y;
    nearest_x_offsets(src.width(), dst_width, 1.0 / inv_scale_x, &mut buffers.x_offsets);
    let (dst_pixels, _) = dst.data_mut().as_chunks_mut::<4>();
    for (y, dst_row) in dst_pixels.chunks_exact_mut(dst_width).enumerate() {
        let (src_row, _) = src.row(nearest_y(y, ify, src.height())).as_chunks::<4>();
        for (dst_pixel, &x) in dst_row.iter_mut().zip(&buffers.x_offsets) {
            *dst_pixel = src_row[x];
        }
    }
}

/// `cv::resize(src, (dst_width, dst_height), interpolation=INTER_NEAREST)`.
pub fn resize_nearest_to(
    src: BgraView,
    dst_width: usize,
    dst_height: usize,
    buffers: &mut ResizeBuffers,
    dst: &mut BgraImage,
) {
    let inv_scale_x = dst_width as f64 / src.width() as f64;
    let inv_scale_y = dst_height as f64 / src.height() as f64;
    resize_nearest(src, dst_width, dst_height, inv_scale_x, inv_scale_y, buffers, dst);
}

/// [`resize_nearest_to`] that only fills the pixels in `spans` (pixel index ranges of the
/// output). The rest of `dst` is left as it was. Masked comparisons only read those pixels.
pub fn resize_nearest_to_spans(
    src: BgraView,
    dst_width: usize,
    dst_height: usize,
    spans: &[Range<usize>],
    buffers: &mut ResizeBuffers,
    dst: &mut BgraImage,
) {
    dst.reshape(dst_width, dst_height);
    let ifx = 1.0 / (dst_width as f64 / src.width() as f64);
    let ify = 1.0 / (dst_height as f64 / src.height() as f64);
    nearest_x_offsets(src.width(), dst_width, ifx, &mut buffers.x_offsets);
    let (dst_pixels, _) = dst.data_mut().as_chunks_mut::<4>();
    for span in spans {
        let mut start = span.start;
        while start < span.end {
            let y = start / dst_width;
            let end = span.end.min((y + 1) * dst_width);
            let (src_row, _) = src.row(nearest_y(y, ify, src.height())).as_chunks::<4>();
            let x0 = start - y * dst_width;
            for (dst_pixel, &x) in dst_pixels[start..end].iter_mut().zip(&buffers.x_offsets[x0..x0 + (end - start)]) {
                *dst_pixel = src_row[x];
            }
            start = end;
        }
    }
}

/// `cv::resize(..., INTER_AREA)` on interleaved 8-bit data with `cn` channels.
///
/// Covers the three paths OpenCV picks between: integer downscale ("area fast"), fractional
/// downscale, and the bilinear emulation used when either axis is upscaled.
#[allow(clippy::too_many_arguments)]
pub fn resize_area(
    src: &[u8],
    src_width: usize,
    src_height: usize,
    cn: usize,
    dst_width: usize,
    dst_height: usize,
    buffers: &mut ResizeBuffers,
    dst: &mut [u8],
) {
    assert_eq!(src.len(), src_width * src_height * cn);
    assert_eq!(dst.len(), dst_width * dst_height * cn);
    if src_width == dst_width && src_height == dst_height {
        dst.copy_from_slice(src);
        return;
    }

    let inv_scale_x = dst_width as f64 / src_width as f64;
    let inv_scale_y = dst_height as f64 / src_height as f64;
    let scale_x = 1.0 / inv_scale_x;
    let scale_y = 1.0 / inv_scale_y;
    let iscale_x = cv_round(scale_x);
    let iscale_y = cv_round(scale_y);
    let is_area_fast =
        (scale_x - iscale_x as f64).abs() < f64::EPSILON && (scale_y - iscale_y as f64).abs() < f64::EPSILON;

    let image = Interleaved { data: src, width: src_width, height: src_height, cn };
    if scale_x >= 1.0 && scale_y >= 1.0 {
        if is_area_fast {
            resize_area_fast(&image, iscale_x as usize, iscale_y as usize, dst_width, &mut buffers.column_sums, dst);
        } else {
            resize_area_fractional(&image, scale_x, scale_y, dst_width, dst_height, buffers, dst);
        }
    } else {
        resize_area_upscale(&image, inv_scale_x, inv_scale_y, scale_x, scale_y, dst_width, dst_height, dst);
    }
}

struct Interleaved<'a> {
    data: &'a [u8],
    width: usize,
    height: usize,
    cn: usize,
}

impl Interleaved<'_> {
    fn row(&self, y: usize) -> &[u8] {
        let stride = self.width * self.cn;
        &self.data[y * stride..(y + 1) * stride]
    }
}

/// `resizeAreaFast_Invoker` with `ResizeAreaFastVec<uchar>`. OpenCV only takes this path when
/// the scales are whole numbers, so every output pixel covers a full `scale_x` by `scale_y` cell.
fn resize_area_fast(
    src: &Interleaved,
    scale_x: usize,
    scale_y: usize,
    dst_width: usize,
    column_sums: &mut Vec<u32>,
    dst: &mut [u8],
) {
    let cn = src.cn;
    debug_assert_eq!((src.width % scale_x, src.height % scale_y), (0, 0));
    let scale = 1.0_f32 / (scale_x * scale_y) as f32;
    // The 2x2 case has its own rounding: (sum + 2) >> 2 instead of cvRound(sum * 0.25f)
    let two_by_two = scale_x == 2 && scale_y == 2 && matches!(cn, 1 | 3 | 4);

    // Sums are exact integers, so adding up each column over the cell's rows first gives the
    // same result as OpenCV's cell by cell loop, and is much faster for large cells
    column_sums.clear();
    column_sums.resize(src.width * cn, 0);
    for (dy, dst_row) in dst.chunks_exact_mut(dst_width * cn).enumerate() {
        column_sums.fill(0);
        for sy in dy * scale_y..(dy + 1) * scale_y {
            for (sum, &value) in column_sums.iter_mut().zip(src.row(sy)) {
                *sum += value as u32;
            }
        }
        for (dx, value) in dst_row.iter_mut().enumerate() {
            let x0 = scale_x * (dx / cn) * cn + dx % cn;
            let sum: u32 = (0..scale_x).map(|sx| column_sums[x0 + sx * cn]).sum();
            *value =
                if two_by_two { ((sum + 2) >> 2) as u8 } else { saturate_u8(cv_round((sum as f32 * scale) as f64)) };
        }
    }
}

struct DecimateAlpha {
    si: usize,
    di: usize,
    alpha: f32,
}

/// `computeResizeAreaTab`.
fn compute_resize_area_tab(src_size: usize, dst_size: usize, cn: usize, scale: f64, tab: &mut Vec<DecimateAlpha>) {
    tab.clear();
    for dx in 0..dst_size {
        let fsx1 = dx as f64 * scale;
        let fsx2 = fsx1 + scale;
        let cell_width = scale.min(src_size as f64 - fsx1);

        let mut sx1 = fsx1.ceil() as i64;
        let mut sx2 = fsx2.floor() as i64;
        sx2 = sx2.min(src_size as i64 - 1);
        sx1 = sx1.min(sx2);

        if sx1 as f64 - fsx1 > 1e-3 {
            tab.push(DecimateAlpha {
                di: dx * cn,
                si: (sx1 - 1) as usize * cn,
                alpha: ((sx1 as f64 - fsx1) / cell_width) as f32,
            });
        }
        for sx in sx1..sx2 {
            tab.push(DecimateAlpha { di: dx * cn, si: sx as usize * cn, alpha: (1.0 / cell_width) as f32 });
        }
        if fsx2 - sx2 as f64 > 1e-3 {
            tab.push(DecimateAlpha {
                di: dx * cn,
                si: sx2 as usize * cn,
                alpha: ((fsx2 - sx2 as f64).min(1.0).min(cell_width) / cell_width) as f32,
            });
        }
    }
}

/// `ResizeArea_Invoker<uchar, float>`.
fn resize_area_fractional(
    src: &Interleaved,
    scale_x: f64,
    scale_y: f64,
    dst_width: usize,
    dst_height: usize,
    buffers: &mut ResizeBuffers,
    dst: &mut [u8],
) {
    let cn = src.cn;
    compute_resize_area_tab(src.width, dst_width, cn, scale_x, &mut buffers.xtab);
    compute_resize_area_tab(src.height, dst_height, 1, scale_y, &mut buffers.ytab);
    let row_len = dst_width * cn;

    // Entry k of every output column side by side, so the columns' sums run as independent
    // chains instead of one long dependent chain per column. Each column still adds its terms
    // in OpenCV's order. Padding adds 0.0 * pixel, which leaves the non-negative sums unchanged.
    let depths = &mut buffers.column_depths;
    depths.clear();
    depths.resize(dst_width, 0);
    for entry in &buffers.xtab {
        depths[entry.di / cn] += 1;
    }
    let max_depth = depths.iter().copied().max().unwrap_or(0);
    let table = &mut buffers.column_table;
    table.clear();
    table.resize(max_depth * dst_width, (0, 0.0));
    depths.fill(0);
    for entry in &buffers.xtab {
        let column = entry.di / cn;
        table[depths[column] * dst_width + column] = (entry.si, entry.alpha);
        depths[column] += 1;
    }

    let (buf, sum) = (&mut buffers.row_buf, &mut buffers.row_sum);
    buf.clear();
    buf.resize(row_len, 0.0);
    sum.clear();
    sum.resize(row_len, 0.0);
    let store = |sum: &[f32], dst: &mut [u8], dy: usize| {
        for (out, &value) in dst[dy * row_len..(dy + 1) * row_len].iter_mut().zip(sum) {
            *out = saturate_u8(cv_round(value as f64));
        }
    };

    let mut prev_dy = buffers.ytab[0].di;
    for y_entry in &buffers.ytab {
        let row = src.row(y_entry.si);
        if cn == 4 {
            let (buf_pixels, _) = buf.as_chunks_mut::<4>();
            for (block, outs) in buf_pixels.chunks_mut(8).enumerate() {
                let mut acc = [[0.0_f32; 4]; 8];
                for layer in table.chunks_exact(dst_width) {
                    let layer = &layer[block * 8..block * 8 + outs.len()];
                    for (out, &(si, alpha)) in acc.iter_mut().zip(layer) {
                        let pixel: [u8; 4] = row[si..si + 4].try_into().unwrap();
                        for c in 0..4 {
                            out[c] += pixel[c] as f32 * alpha;
                        }
                    }
                }
                outs.copy_from_slice(&acc[..outs.len()]);
            }
        } else {
            buf.fill(0.0);
            for layer in table.chunks_exact(dst_width) {
                for (out, &(si, alpha)) in buf.chunks_exact_mut(cn).zip(layer) {
                    for c in 0..cn {
                        out[c] += row[si + c] as f32 * alpha;
                    }
                }
            }
        }

        let beta = y_entry.alpha;
        if y_entry.di != prev_dy {
            store(sum, dst, prev_dy);
            for (s, &b) in sum.iter_mut().zip(buf.iter()) {
                *s = beta * b;
            }
            prev_dy = y_entry.di;
        } else {
            for (s, &b) in sum.iter_mut().zip(buf.iter()) {
                *s += beta * b;
            }
        }
    }
    store(sum, dst, prev_dy);
}

const INTER_RESIZE_COEF_SCALE: f32 = 2048.0;

/// Coefficients OpenCV uses for INTER_AREA when an axis is upscaled ("area_mode" bilinear).
fn area_upscale_coefficients(
    dst_size: usize,
    src_size: usize,
    scale: f64,
    inv_scale: f64,
) -> (Vec<usize>, Vec<[i32; 2]>, usize) {
    let mut max = dst_size;
    let mut offsets = Vec::with_capacity(dst_size);
    let mut coefficients = Vec::with_capacity(dst_size);
    for d in 0..dst_size {
        let mut s = (d as f64 * scale).floor() as i64;
        let mut f = ((d + 1) as f64 - (s + 1) as f64 * inv_scale) as f32;
        f = if f <= 0.0 { 0.0 } else { f - f.floor() };
        if s + 1 >= src_size as i64 {
            max = max.min(d);
            if s >= src_size as i64 - 1 {
                f = 0.0;
                s = src_size as i64 - 1;
            }
        }
        offsets.push(s as usize);
        let c0 = cv_round(((1.0 - f) * INTER_RESIZE_COEF_SCALE) as f64).clamp(i16::MIN as i64, i16::MAX as i64);
        let c1 = cv_round((f * INTER_RESIZE_COEF_SCALE) as f64).clamp(i16::MIN as i64, i16::MAX as i64);
        coefficients.push([c0 as i32, c1 as i32]);
    }
    (offsets, coefficients, max)
}

/// `resizeGeneric_` with `HResizeLinear`/`VResizeLinear` for 8-bit, in INTER_AREA's area mode.
/// Only reached for split images under 8 pixels on a side, so it isn't tuned.
#[allow(clippy::too_many_arguments)]
fn resize_area_upscale(
    src: &Interleaved,
    inv_scale_x: f64,
    inv_scale_y: f64,
    scale_x: f64,
    scale_y: f64,
    dst_width: usize,
    dst_height: usize,
    dst: &mut [u8],
) {
    let cn = src.cn;
    let (x_offsets, x_coefficients, x_max) = area_upscale_coefficients(dst_width, src.width, scale_x, inv_scale_x);
    let (y_offsets, y_coefficients, _) = area_upscale_coefficients(dst_height, src.height, scale_y, inv_scale_y);

    let horizontal = |sy: usize| -> Vec<i32> {
        let row = src.row(sy);
        let mut out = Vec::with_capacity(dst_width * cn);
        for dx in 0..dst_width {
            let sx = x_offsets[dx] * cn;
            for c in 0..cn {
                out.push(if dx < x_max {
                    let [a0, a1] = x_coefficients[dx];
                    row[sx + c] as i32 * a0 + row[sx + c + cn] as i32 * a1
                } else {
                    row[sx + c] as i32 * INTER_RESIZE_COEF_SCALE as i32
                });
            }
        }
        out
    };

    let clip = |y: usize| y.min(src.height - 1);
    for (dy, dst_row) in dst.chunks_exact_mut(dst_width * cn).enumerate() {
        let row0 = horizontal(clip(y_offsets[dy]));
        let row1 = horizontal(clip(y_offsets[dy] + 1));
        let [b0, b1] = y_coefficients[dy];
        for ((out, &s0), &s1) in dst_row.iter_mut().zip(&row0).zip(&row1) {
            let value = ((b0 * (s0 >> 4)) >> 16) + ((b1 * (s1 >> 4)) >> 16) + 2;
            *out = saturate_u8((value >> 2) as i64);
        }
    }
}

/// `cv::resize(..., INTER_LINEAR_EXACT)` for 8-bit images, using OpenCV's
/// `ufixedpoint16` coefficients (8 fractional bits) and `ufixedpoint32` accumulation.
#[allow(clippy::too_many_arguments)]
pub fn resize_linear_exact(
    src: &[u8],
    src_width: usize,
    src_height: usize,
    cn: usize,
    dst_width: usize,
    dst_height: usize,
    buffers: &mut ResizeBuffers,
    dst: &mut [u8],
) {
    assert_eq!(src.len(), src_width * src_height * cn);
    assert_eq!(dst.len(), dst_width * dst_height * cn);
    if src_width == dst_width && src_height == dst_height {
        dst.copy_from_slice(src);
        return;
    }

    let ResizeBuffers { x_axis: x, y_axis: y, lines, .. } = buffers;
    x.compute(dst_width as f64 / src_width as f64, src_width, dst_width);
    y.compute(dst_height as f64 / src_height as f64, src_height, dst_height);
    let image = Interleaved { data: src, width: src_width, height: src_height, cn };

    // Every source row goes through the horizontal pass once, instead of twice per output row
    let row_len = dst_width * cn;
    lines.clear();
    lines.resize(src_height * row_len, 0);
    for (sy, out) in lines.chunks_exact_mut(row_len).enumerate() {
        let row = image.row(sy);
        for (dx, pixel) in out.chunks_exact_mut(cn).enumerate() {
            for (c, value) in pixel.iter_mut().enumerate() {
                *value = if dx < x.min {
                    (row[c] as u16) << 8
                } else if dx < x.max {
                    let sx = x.offsets[dx] * cn + c;
                    let [m0, m1] = x.coefficients[dx];
                    // ufixedpoint16 products and sums saturate, but can't overflow here
                    (m0 as u32 * row[sx] as u32 + m1 as u32 * row[sx + cn] as u32).min(u16::MAX as u32) as u16
                } else {
                    (row[(src_width - 1) * cn + c] as u16) << 8
                };
            }
        }
    }

    let line = |sy: usize| &lines[sy * row_len..(sy + 1) * row_len];
    let to_u8 = |value: u16| saturate_u8(((value as u32 + 128) >> 8) as i64);
    for (dy, dst_row) in dst.chunks_exact_mut(row_len).enumerate() {
        if dy < y.min || dy >= y.max {
            for (out, &value) in dst_row.iter_mut().zip(line(if dy < y.min { 0 } else { src_height - 1 })) {
                *out = to_u8(value);
            }
            continue;
        }
        let sy = y.offsets[dy];
        let [m0, m1] = y.coefficients[dy];
        for ((out, &v0), &v1) in dst_row.iter_mut().zip(line(sy)).zip(line(sy + 1)) {
            let sum = (v0 as u32 * m0 as u32).saturating_add(v1 as u32 * m1 as u32);
            *out = saturate_u8((sum.saturating_add(1 << 15) >> 16) as i64);
        }
    }
}

/// `interpolationLinear<uchar>` from resize.cpp.
#[derive(Default)]
struct LinearExactAxis {
    offsets: Vec<usize>,
    coefficients: Vec<[u16; 2]>,
    min: usize,
    max: usize,
}

impl LinearExactAxis {
    fn compute(&mut self, inv_scale: f64, src_size: usize, dst_size: usize) {
        let scale = 1.0 / inv_scale;
        self.offsets.clear();
        self.offsets.resize(dst_size, 0);
        self.coefficients.clear();
        self.coefficients.resize(dst_size, [0, 0]);
        self.min = 0;
        self.max = dst_size;
        for d in 0..dst_size {
            let fval = scale * (d as f64 + 0.5) - 0.5;
            let ival = fval.floor() as i64;
            if ival >= 0 && src_size > 1 {
                if ival < src_size as i64 - 1 {
                    self.offsets[d] = ival as usize;
                    let c1 = cv_round((fval - ival as f64) * 256.0) as u16;
                    self.coefficients[d] = [256_u16.saturating_sub(c1), c1];
                } else {
                    self.offsets[d] = src_size - 1;
                    self.max = self.max.min(d);
                }
            } else {
                self.min = self.min.max(d + 1);
            }
        }
    }
}

/// `cv::cvtColor(..., COLOR_BGR2GRAY)` for 8-bit, with 3 or 4 source channels.
pub fn bgr_to_gray(src: &[u8], cn: usize, dst: &mut [u8]) {
    const SHIFT: u32 = 15;
    const BY: u32 = 3735;
    const GY: u32 = 19235;
    const RY: u32 = 9798;
    for (pixel, out) in src.chunks_exact(cn).zip(dst.iter_mut()) {
        let sum = pixel[0] as u32 * BY + pixel[1] as u32 * GY + pixel[2] as u32 * RY + (1 << (SHIFT - 1));
        *out = (sum >> SHIFT) as u8;
    }
}

/// Stretches `view` to `width` x `height` with bilinear filtering, as RGBA for showing on
/// screen. Not an OpenCV port: it's for the live preview, so it uses 8-bit fixed point weights.
pub fn stretch_to_rgba(view: BgraView, width: usize, height: usize, out: &mut [[u8; 4]]) {
    debug_assert_eq!(out.len(), width * height);
    if view.is_empty() {
        out.fill([0; 4]);
        return;
    }
    // Where each output pixel's center falls in the source, as two neighbors and the weight of
    // the second in 1/256
    let taps = |out_size: usize, size: usize| -> Vec<(usize, usize, u32)> {
        let scale = size as f32 / out_size as f32;
        (0..out_size)
            .map(|i| {
                let at = ((i as f32 + 0.5) * scale - 0.5).clamp(0.0, (size - 1) as f32);
                let first = at as usize;
                (first, (first + 1).min(size - 1), ((at - first as f32) * 256.0).round() as u32)
            })
            .collect()
    };
    let columns = taps(width, view.width());
    for ((y0, y1, fy), row) in taps(height, view.height()).into_iter().zip(out.chunks_exact_mut(width.max(1))) {
        let (top, bottom) = (view.row(y0), view.row(y1));
        for (&(x0, x1, fx), pixel) in columns.iter().zip(row) {
            let mix = |channel: usize| {
                let blend = |source: &[u8]| {
                    u32::from(source[x0 * 4 + channel]) * (256 - fx) + u32::from(source[x1 * 4 + channel]) * fx
                };
                ((blend(top) * (256 - fy) + blend(bottom) * fy + (1 << 15)) >> 16) as u8
            };
            *pixel = [mix(2), mix(1), mix(0), mix(3)];
        }
    }
}
