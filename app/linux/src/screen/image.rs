//! The image sent to the model is the only coordinate space.

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bounds {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScreenPoint {
    pub x: f64,
    pub y: f64,
}

pub struct Encoded {
    pub data_url: String,
    pub width: i32,
    pub height: i32,
}

pub fn image_point(x: f64, y: f64, width: i32, height: i32, bounds: Bounds) -> Option<ScreenPoint> {
    let inside = x.is_finite()
        && y.is_finite()
        && width > 0
        && height > 0
        && x >= 0.0
        && y >= 0.0
        && x < f64::from(width)
        && y < f64::from(height);
    inside.then(|| ScreenPoint {
        x: bounds.x + x * bounds.w / f64::from(width),
        y: bounds.y + y * bounds.h / f64::from(height),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCREEN: Bounds = Bounds {
        x: 100.0,
        y: 50.0,
        w: 2560.0,
        h: 1440.0,
    };

    #[test]
    fn a_point_of_the_image_maps_onto_the_captured_screen() {
        assert_eq!(
            image_point(640.0, 360.0, 1280, 720, SCREEN),
            Some(ScreenPoint {
                x: 1380.0,
                y: 770.0
            })
        );
        assert_eq!(
            image_point(0.0, 0.0, 1280, 720, SCREEN),
            Some(ScreenPoint { x: 100.0, y: 50.0 })
        );
        assert_eq!(
            image_point(1279.5, 719.5, 1280, 720, SCREEN),
            Some(ScreenPoint {
                x: 2659.0,
                y: 1489.0
            })
        );
    }

    #[test]
    fn anything_outside_the_image_has_no_screen_point() {
        for (x, y, width, height) in [
            (1280.0, 10.0, 1280, 720),
            (10.0, 720.0, 1280, 720),
            (-0.5, 10.0, 1280, 720),
            (10.0, -0.5, 1280, 720),
            (f64::NAN, 10.0, 1280, 720),
            (10.0, f64::NAN, 1280, 720),
            (f64::INFINITY, 10.0, 1280, 720),
            (10.0, f64::NEG_INFINITY, 1280, 720),
            (0.0, 0.0, 0, 720),
            (0.0, 0.0, 1280, 0),
            (0.0, 0.0, -1, 720),
        ] {
            assert_eq!(image_point(x, y, width, height, SCREEN), None, "{x},{y}");
        }
    }
}
