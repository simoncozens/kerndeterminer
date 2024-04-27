use babelfont::{Font, Layer, Master};
use core::cmp::Ordering;
use kurbo::{Affine, BezPath, CubicBez, ParamCurve, ParamCurveNearest, PathSeg, Vec2};
use pyo3::prelude::*;
use std::collections::HashMap;
use {env_logger, log};

#[pyclass]
struct KernDeterminer {
    font: Font,
    layer_paths: HashMap<Layer, Vec<BezPath>>,
}

#[pymethods]
impl KernDeterminer {
    #[new]
    fn new(filename: String) -> Self {
        env_logger::init();
        let mut font = babelfont::load(&filename).expect("Couldn't load font");
        for glyph_index in 0..font.glyphs.0.len() {
            let mut decomposed_layers = Vec::new();
            if let Some(glyph) = font.glyphs.get_by_index(glyph_index) {
                for layer in glyph.layers.iter() {
                    let decomposed_layer = layer.decomposed_components(&font);
                    decomposed_layers.push(decomposed_layer);
                }
            } else {
                continue;
            }

            if let Some(glyph) = font.glyphs.get_by_index_mut(glyph_index) {
                for (layer, decomposed_paths) in glyph.layers.iter_mut().zip(decomposed_layers) {
                    for path in decomposed_paths {
                        layer.push_path(path);
                    }
                    layer.clear_components();
                }
            }
        }
        KernDeterminer {
            font,
            layer_paths: HashMap::new(),
        }
    }

    fn determine_kern(
        &self,
        left_glyph: String,
        right_glyph: String,
        master_name: String,
        target_distance: f32,
        height: i32,
        max_tuck: f32,
    ) -> PyResult<f32> {
        let master = self
            .font
            .master(&master_name)
            .unwrap_or_else(|| panic!("Couldn't find master {:}", master_name));
        Ok(_determine_kern(
            &self.font,
            master,
            &left_glyph,
            &right_glyph,
            target_distance,
            height,
            max_tuck,
        ))
    }
}

fn _determine_kern(
    font: &Font,
    master: &Master,
    left_glyph: &str,
    right_glyph: &str,
    target_distance: f32,
    height: i32,
    max_tuck: f32,
) -> f32 {
    let layer_1 = font
        .master_layer_for(left_glyph, master)
        .unwrap_or_else(|| panic!("{}", format!("Couldn't find glyph {:}", left_glyph)));
    let layer_2 = font
        .master_layer_for(right_glyph, master)
        .unwrap_or_else(|| panic!("{}", format!("Couldn't find glyph {:}", right_glyph)));

    // Get exit anchor
    let lexit = layer_1
        .anchors
        .iter()
        .find(|a| a.name == "exit")
        .map(|a| a.y)
        .unwrap_or(0);
    let height = if height > 0 { height - lexit } else { height };
    let mut minimum_possible = -1000.0;
    if max_tuck != 0.0 {
        let maximum_width = layer_1.width as f32 * max_tuck;
        let left_edge = (-layer_2.lsb().expect("Oops")).min(0.0);
        minimum_possible = left_edge - maximum_width;
    }
    let mut iterations = 0;
    let mut kern = 0.0;
    let mut min_distance = -9999.0;
    let left_paths: Vec<BezPath> = layer_1
        .paths()
        .map(|x| x.to_kurbo().expect("Couldn't convert paths?!"))
        .collect();
    let right_paths: Vec<BezPath> = layer_2
        .paths()
        .map(|x| x.to_kurbo().expect("Couldn't convert paths?!"))
        .collect();

    while iterations < 10 && (target_distance - min_distance).abs() > 10.0 {
        if let Some(md) = _path_distance(
            &left_paths,
            &right_paths,
            kern + layer_1.width as f32,
            height as f32,
        ) {
            log::debug!("With kern of {:?}, distance was {:?}", kern, md);
            min_distance = md;
            kern += target_distance - min_distance;
            if kern < minimum_possible {
                return minimum_possible;
            }
            iterations += 1;
        } else {
            return minimum_possible;
        }
    }
    kern
}

fn _path_distance(
    left_paths: &[BezPath],
    right_paths: &[BezPath],
    x_offset: f32,
    y_offset: f32,
) -> Option<f32> {
    let offset1 = Affine::translate(Vec2 {
        x: 0.0,
        y: y_offset.into(),
    });
    let offset2 = Affine::translate(Vec2 {
        x: x_offset as f64,
        y: 0.0,
    });
    let mut min_distance: Option<f64> = None;
    for p1 in left_paths {
        let moved_p1 = offset1 * p1;
        for p2 in right_paths {
            let moved_p2 = offset2 * p2;
            let d = min_distance_bezpath(&moved_p1, &moved_p2);
            log::debug!("  d={:?}", d);
            if min_distance.is_none() || d < min_distance.unwrap() {
                log::debug!("    (new record)");
                min_distance = Some(d)
            } else {
                log::debug!("    (ignored)");
            }
        }
    }
    min_distance.map(|x| x as f32)
}

#[pymodule]
fn kerndeterminer(_py: Python, m: &PyModule) -> PyResult<()> {
    m.add_class::<KernDeterminer>()?;
    Ok(())
}

fn min_distance_bezpath(one: &BezPath, other: &BezPath) -> f64 {
    let segs1 = one.segments();
    let mut best_pair: Option<(f64, kurbo::PathSeg, kurbo::PathSeg)> = None;
    for s1 in segs1 {
        let p1 = vec![s1.eval(0.0), s1.eval(0.5), s1.eval(1.0)];
        for s2 in other.segments() {
            let p2 = vec![s2.eval(0.0), s2.eval(0.5), s2.eval(1.0)];
            let dist = p1
                .iter()
                .zip(p2.iter())
                .map(|(a, b)| a.distance(*b))
                .min_by(|a, b| a.partial_cmp(b).unwrap_or(Ordering::Less))
                .unwrap();
            if let Some((best, _, _)) = best_pair {
                if dist > best {
                    continue;
                }
            }
            best_pair = Some((dist, s1, s2));
        }
    }
    if let Some((_, s1, s2)) = best_pair {
        log::debug!("Best pair was {:?}, {:?}", s1, s2);
        match (s1, s2) {
            (PathSeg::Line(l1), PathSeg::Line(l2)) => line_line_dist(l1, l2),
            (PathSeg::Line(l1), PathSeg::Cubic(c2)) => line_curve_dist(l1, c2),
            (PathSeg::Cubic(c1), PathSeg::Line(l2)) => line_curve_dist(l2, c1),
            (PathSeg::Cubic(c1), PathSeg::Cubic(c2)) => s1.min_dist(s2, 0.5).distance,
            _ => panic!("Unusual configuration"),
        }
    } else {
        f64::MAX
    }
}

fn line_line_dist(l1: kurbo::Line, l2: kurbo::Line) -> f64 {
    let a = l1.nearest(l2.p0, 1.0).distance_sq;
    let b = l1.nearest(l2.p1, 1.0).distance_sq;
    let c = l2.nearest(l1.p0, 1.0).distance_sq;
    let d = l2.nearest(l1.p1, 1.0).distance_sq;
    (a.min(b).min(c).min(d)).sqrt()
}

fn line_curve_dist(l1: kurbo::Line, c1: kurbo::CubicBez) -> f64 {
    let t = [0.0, 0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9];
    t.iter()
        .map(|x| c1.nearest(l1.eval(*x), 1.0).distance_sq)
        .reduce(|a, b| a.min(b))
        .unwrap_or(f64::MAX)
        .sqrt()
}

// #[cfg(test)]
// mod tests {
//     use super::*;

//     #[test]
//     fn test_sanity() {
//         let determiner = KernDeterminer::new("GSN.glyphs".to_string());
//         let kern = determiner
//             .determine_kern(
//                 "BEi9".to_string(),
//                 "SINus1".to_string(),
//                 "Light Ultra".to_string(),
//                 150.0,
//                 0,
//                 0.65,
//             )
//             .unwrap();
//         assert!((kern - (-137.15)).abs() < 0.01);
//     }
// }
