//! Procedural app icon: no downloaded assets or runtime image tooling.
//! A macOS-style squircle on the blue-violet brand gradient, holding a folded
//! page with a highlighted line and a signature stroke.

type Rgba = [f32; 4];

fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}
/// Premultiplied "source over destination".
fn over(dst: Rgba, src: Rgba) -> Rgba {
    std::array::from_fn(|i| src[i] + dst[i] * (1.0 - src[3]))
}
fn paint(rgb: [f32; 3], alpha: f32) -> Rgba {
    [
        rgb[0] / 255. * alpha,
        rgb[1] / 255. * alpha,
        rgb[2] / 255. * alpha,
        alpha,
    ]
}
/// Superellipse radius: 1.0 on the squircle outline.
fn squircle(x: f32, y: f32, cx: f32, cy: f32, half: f32) -> f32 {
    let n = 5.0;
    (((x - cx).abs() / half).powf(n) + ((y - cy).abs() / half).powf(n)).powf(1.0 / n)
}
/// Signed distance to a rounded rectangle.
fn rounded_rect(x: f32, y: f32, min: [f32; 2], max: [f32; 2], r: f32) -> f32 {
    let cx = (min[0] + max[0]) / 2.;
    let cy = (min[1] + max[1]) / 2.;
    let qx = (x - cx).abs() - ((max[0] - min[0]) / 2. - r);
    let qy = (y - cy).abs() - ((max[1] - min[1]) / 2. - r);
    qx.max(0.).hypot(qy.max(0.)) + qx.max(qy).min(0.) - r
}

const BODY: f32 = 0.402; // Half-size on Apple's 824/1024 icon grid.
const SHEET_MIN: [f32; 2] = [0.315, 0.215];
const SHEET_MAX: [f32; 2] = [0.685, 0.785];
const FOLD: f32 = 0.11;

fn sample(x: f32, y: f32) -> Rgba {
    let mut c = [0.0; 4];
    // Ambient drop shadow under the squircle.
    let k = squircle(x, y - 0.018, 0.5, 0.5, BODY);
    c = over(
        c,
        paint([10., 20., 60.], 0.3 * (1.0 - smoothstep(0.93, 1.1, k))),
    );
    if squircle(x, y, 0.5, 0.5, BODY) > 1.0 {
        return c;
    }
    // Brand gradient, with a soft light from the top.
    let t = ((x - 0.098) + (y - 0.098)) / (4.0 * BODY);
    let a = [64., 140., 255.];
    let b = [128., 84., 250.];
    let mut body: [f32; 3] = std::array::from_fn(|i| a[i] + (b[i] - a[i]) * t.clamp(0., 1.));
    let glow = (1.0 - ((y - 0.098) / (2.0 * BODY))).clamp(0., 1.).powi(2) * 0.12;
    for v in &mut body {
        *v += (255. - *v) * glow;
    }
    c = over(c, paint(body, 1.0));
    // Page shadow, then the page with its top-right corner folded.
    let shadow = rounded_rect(x, y - 0.014, SHEET_MIN, SHEET_MAX, 0.03);
    c = over(
        c,
        paint(
            [20., 20., 90.],
            0.22 * (1.0 - smoothstep(-0.01, 0.035, shadow)),
        ),
    );
    let corner = [SHEET_MAX[0] - FOLD, SHEET_MIN[1]];
    let cut = (x - corner[0]) - (y - corner[1]) > FOLD;
    if rounded_rect(x, y, SHEET_MIN, SHEET_MAX, 0.03) <= 0.0 && !cut {
        c = over(c, paint([255., 255., 255.], 1.0));
        let in_fold =
            x >= corner[0] && y <= corner[1] + FOLD && (x - corner[0]) <= (y - corner[1]) + 0.002;
        if in_fold {
            let shade = 1.0 - smoothstep(0.0, FOLD, (y - corner[1]) - (x - corner[0]));
            c = over(c, paint([200., 208., 255.], 0.75 + 0.25 * shade));
        }
        // Highlighter band behind the first line.
        if rounded_rect(x, y, [0.358, 0.39], [0.642, 0.45], 0.012) <= 0.0 {
            c = over(c, paint([255., 206., 48.], 0.95));
        }
        // Text lines with rounded ends.
        for (line_y, end) in [(0.42, 0.62), (0.505, 0.62), (0.59, 0.53)] {
            let d = rounded_rect(x, y, [0.38, line_y - 0.012], [end, line_y + 0.012], 0.012);
            if d <= 0.0 {
                c = over(c, paint([150., 160., 228.], 1.0));
            }
        }
        // A signature stroke across the foot of the page.
        let curve = |u: f32| 0.69 + 0.03 * (u * 11.0).sin() * (1.0 - 0.6 * u);
        let (start, span, width) = (0.38, 0.24, 0.009);
        let u = (x - start) / span;
        let on_curve = (0.0..=1.0).contains(&u) && {
            let slope = 0.03 * 11.0 / span * (u * 11.0).cos() * (1.0 - 0.6 * u);
            (y - curve(u)).abs() / (1.0 + slope * slope).sqrt() < width
        };
        let on_cap = [0.0, 1.0]
            .iter()
            .any(|&u| (x - start - u * span).hypot(y - curve(u)) < width);
        if on_curve || on_cap {
            c = over(c, paint([66., 104., 245.], 1.0));
        }
    }
    c
}

pub fn rgba(size: u32) -> Vec<u8> {
    const SS: u32 = 4;
    let mut pixels = Vec::with_capacity((size * size * 4) as usize);
    for py in 0..size {
        for px in 0..size {
            let mut sum = [0.0_f32; 4];
            for sy in 0..SS {
                for sx in 0..SS {
                    let x = (px as f32 + (sx as f32 + 0.5) / SS as f32) / size as f32;
                    let y = (py as f32 + (sy as f32 + 0.5) / SS as f32) / size as f32;
                    let s = sample(x, y);
                    for i in 0..4 {
                        sum[i] += s[i];
                    }
                }
            }
            let n = (SS * SS) as f32;
            let alpha = sum[3] / n;
            let unpremultiply = |v: f32| {
                if alpha > 0.0 {
                    (v / n / alpha * 255.0).round().clamp(0.0, 255.0) as u8
                } else {
                    0
                }
            };
            pixels.extend([
                unpremultiply(sum[0]),
                unpremultiply(sum[1]),
                unpremultiply(sum[2]),
                (alpha * 255.0).round() as u8,
            ]);
        }
    }
    pixels
}
