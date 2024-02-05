use crate::thermal;

use scarlet::prelude::RGBColor;

#[derive(Debug)]
pub enum Flip {
    Horizontal,
    Vertical,
}

impl<T: image2::Type, C: image2::Color, U: image2::Type, D: image2::Color>
    image2::Filter<T, C, U, D> for Flip
{
    fn compute_at(
        &self,
        pt: image2::Point,
        input: &image2::Input<T, C>,
        dest: &mut image2::DataMut<U, D>,
    ) {
        let px = match *self {
            Flip::Horizontal => input.get_pixel([input.images[0].height() - 1 - pt.x, pt.y], None),
            Flip::Vertical => input.get_pixel([pt.x, input.images[0].width() - 1 - pt.y], None),
        };
        px.copy_to_slice(dest);
    }
}

pub fn generate_black_image(width: usize, height: usize) -> thermal::RgbImage {
    let mut imgbuf = thermal::RgbImage::new([width, height]);
    let black = [0, 0, 0];

    imgbuf.each_pixel_mut(|_pt, pixel| {
        pixel.copy_from_slice(black);
    });

    imgbuf
}

pub fn map_to_scaled_value(input: u16, min: u16, max: u16, color_range: u8) -> f64 {
    let color_range = f64::from(color_range) / 100.0;
    let value = f64::from(input - min) / f64::from(max - min);

    ((1.0 - color_range) / 2.0) + value * color_range
}

pub fn generate_colormap_image(
    width: usize,
    height: usize,
    cmap: &(dyn scarlet::colormap::ColorMap<scarlet::color::RGBColor> + Sync),
    color_range: u8,
) -> thermal::RgbImage {
    let mut imgbuf = thermal::RgbImage::new([width, height]);

    imgbuf.each_pixel_mut(|pt, pixel| {
        let scaled_value = map_to_scaled_value(
            u16::try_from(pt.x).unwrap(),
            0,
            u16::try_from(width - 1).unwrap(),
            color_range,
        );
        let color: RGBColor = cmap.transform_single(scaled_value);

        pixel.copy_from_slice([color.int_r(), color.int_g(), color.int_b()]);
    });

    imgbuf
}
