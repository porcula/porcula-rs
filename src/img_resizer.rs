use image::GenericImageView;

pub fn resize(src: &[u8], width: u32, height: u32) -> Result<Vec<u8>, String> {
    //catch assert in core when decoding broken image
    let decoded = std::panic::catch_unwind(|| image::load_from_memory(src));
    match decoded {
        Ok(Ok(src)) => {
            //Lanczos3 is slow
            //let dst = src.resize(width, height, image::imageops::FilterType::Lanczos3);
            let dst = src.thumbnail(width, height);
            let mut buf = Vec::<u8>::new();
            let mut enc = image::jpeg::JpegEncoder::new(&mut buf);
            let dim = dst.dimensions();
            match enc.encode(&dst.into_rgb8(), dim.0, dim.1, image::ColorType::Rgb8) {
                Ok(()) => Ok(buf),
                Err(e) => Err(e.to_string()),
            }
        }
        Ok(Err(e)) => Err(e.to_string()),
        Err(_) => Err("image decode error".to_string()),
    }
}
