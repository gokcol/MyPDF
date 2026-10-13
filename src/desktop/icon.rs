//! Small procedural icon: no downloaded assets or runtime image tooling.
pub fn rgba(size: u32) -> Vec<u8> {
    fn polygon(x: f32, y: f32, vertices: &[(f32, f32)]) -> bool {
        let mut inside = false;
        for (a, b) in vertices
            .iter()
            .zip(vertices.iter().cycle().skip(1))
            .take(vertices.len())
        {
            if (a.1 > y) != (b.1 > y) && x < (b.0 - a.0) * (y - a.1) / (b.1 - a.1) + a.0 {
                inside = !inside;
            }
        }
        inside
    }
    let sheet = [
        (0.29, 0.20),
        (0.60, 0.20),
        (0.76, 0.36),
        (0.76, 0.81),
        (0.29, 0.81),
    ];
    let fold = [(0.60, 0.20), (0.60, 0.36), (0.76, 0.36)];
    let mut pixels = Vec::with_capacity((size * size * 4) as usize);
    for py in 0..size {
        for px in 0..size {
            let mut sum = [0_u32; 4];
            for sy in 0..4 {
                for sx in 0..4 {
                    let x = (px as f32 + (sx as f32 + 0.5) / 4.0) / size as f32;
                    let y = (py as f32 + (sy as f32 + 0.5) / 4.0) / size as f32;
                    let dx = (x - 0.5).abs() - 0.30;
                    let dy = (y - 0.5).abs() - 0.30;
                    let color = if dx.max(0.0).hypot(dy.max(0.0)) > 0.18 {
                        [0, 0, 0, 0]
                    } else if polygon(x, y, &fold) {
                        [200, 225, 213, 255]
                    } else if polygon(x, y, &sheet) {
                        if x > 0.37
                            && x < 0.67
                            && ((0.47..0.50).contains(&y) || (0.57..0.60).contains(&y))
                            || x > 0.37 && x < 0.58 && (0.67..0.70).contains(&y)
                        {
                            [28, 109, 89, 255]
                        } else {
                            [249, 249, 245, 255]
                        }
                    } else {
                        [28, 109, 89, 255]
                    };
                    for i in 0..4 {
                        sum[i] += color[i];
                    }
                }
            }
            pixels.extend(sum.map(|v| (v / 16) as u8));
        }
    }
    pixels
}
