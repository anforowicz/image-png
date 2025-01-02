use anyhow::Result;
use png::Decoder;
use std::fs::File;
use std::path::Path;

fn handle_arg(arg: &str) -> Result<()> {
    let path = Path::new(&arg);
    let file = File::open(&path)?;
    let decoder = Decoder::new(file);
    let mut reader = decoder.read_info()?;
    let info = reader.info();
    let image_pixels = info.width * info.height;
    let bits_per_sample = info.bit_depth as u8 as usize;
    let samples_per_pixel = info.color_type.samples();
    let bytes_per_pixel = samples_per_pixel * bits_per_sample / 8;
    let image_bytes = image_pixels as usize * bytes_per_pixel;
    let apng_frames_count = info.animation_control.as_ref().map(|actl| actl.num_frames).unwrap_or(1);
    println!("{{");
    println!("    \"path\": \"{arg}\",");
    println!("    \"bits_per_sample\": {bits_per_sample},");
    println!("    \"color_type\": {},", info.color_type as u8);
    println!("    \"interlaced\": {},", info.interlaced as u8);
    println!("    \"apng_frames_count\": {apng_frames_count},");
    println!("    \"image_bytes\": {image_bytes},");
    let mut buf = vec![0; reader.output_buffer_size()];
    reader.next_frame(&mut buf)?;
    println!("    \"file_bytes\": {}", std::fs::metadata(path)?.len());
    println!("}},");
    Ok(())
}

fn main() {
    for arg in std::env::args().skip(1) {
        let _ignore = handle_arg(&arg);
    }
}
