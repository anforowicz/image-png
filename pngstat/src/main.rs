use anyhow::Result;
use png::Decoder;
use std::collections::HashMap;
use std::fs::File;
use std::io::Write;
use std::path::Path;

fn load_csv(path: &Path) -> Result<HashMap<String, Vec<String>>> {
    let bytes = std::fs::read(path)?;
    let chars = String::from_utf8_lossy(&bytes);
    let mut result = HashMap::new();
    for line in chars.lines() {
        let entries = line.trim().split(",").map(|s| s.to_string()).collect::<Vec<String>>();
        result.insert(entries[0].clone(), entries);
    }
    Ok(result)
}

fn save_csv(path: &Path, hashmap: HashMap<String, Vec<String>>) -> Result<()> {
    let mut file = File::create(&path)?;
    for entries in hashmap.values() {
        for entry in entries.iter() {
            write!(&mut file, "{},", entry)?;
        }
        writeln!(&mut file)?;
    }
    Ok(())
}

fn handle_file(arg: &str, entries: &mut Vec<String>) -> Result<()> {
    let blah = format!("../tests/benches/{}", &arg);
    let path = Path::new(&blah);
    let file = File::open(&path)?;
    let decoder = Decoder::new(file);
    let mut reader = decoder.read_info()?;
    let mut buf = vec![0; reader.output_buffer_size()];
    let frame_info = reader.next_frame(&mut buf)?;
    let info = reader.info();
    let image_pixels = info.width * info.height;
    let bits_per_sample = info.bit_depth as u8 as usize;
    let samples_per_pixel = info.color_type.samples();
    let bytes_per_pixel = samples_per_pixel * bits_per_sample / 8;
    let _image_bytes = image_pixels as usize * bytes_per_pixel;
    entries.push(bits_per_sample.to_string());
    entries.push((info.color_type as u8).to_string());
    entries.push((info.interlaced as u8).to_string());
    entries.push(frame_info.buffer_size().to_string());
    entries.push(reader.idat_size().to_string());
    entries.push(std::fs::metadata(path)?.len().to_string());
    Ok(())
}

fn main() -> Result<()> {
    // Invoke with `path-to-csv.csv`.  The 1st column should be the filename
    // (it will be opened from '../tests/benches/' directory - see `handle_file` above).
    let filename = std::env::args().skip(1).next().unwrap().to_string();
    let path = Path::new(&filename);
    let mut hashmap = load_csv(path)?;
    for (key, values) in hashmap.iter_mut() {
        handle_file(key, values)?;
    }
    save_csv(Path::new("ztest-stats.csv"), hashmap)?;
    Ok(())
}
