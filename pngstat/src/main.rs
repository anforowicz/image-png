use anyhow::Result;
use png::Decoder;
use std::collections::HashMap;
use std::fs::File;
use std::io::Write;
use std::path::Path;

fn load_csv(hashmap: &mut HashMap<String, Vec<String>>, path: &Path) -> Result<()> {
    let filename = path.file_name().unwrap().to_string_lossy();
    if let Some(val) = hashmap.get_mut("!header") {
        val.push(filename.to_string());
    } else {
        panic!("No header");
    };

    let bytes = std::fs::read(path)?;
    let chars = String::from_utf8_lossy(&bytes);
    let mut png_filename = "blah".to_string();
    for line in chars.lines() {
        if line.contains("Warming up for") {
            assert!(line.starts_with("Benchmarking "));
            let line = std::str::from_utf8(&line.as_bytes()[13..]).unwrap();
            match line.find("decode/ztest") {
                None => png_filename = "skip_this".to_string(),
                Some(index) => {
                    let line = std::str::from_utf8(&line.as_bytes()[(index + 7)..]).unwrap();
                    let colon = line.find(':').unwrap();
                    png_filename = line[..colon].to_string();
                }
            }
        }
        match line.find("png  time:") {
            None => (),
            Some(index) => {
                let line = std::str::from_utf8(&line.as_bytes()[index..]).unwrap();
                let line = std::str::from_utf8(&line.as_bytes()[line.find('[').unwrap() + 1..]).unwrap();
                let line = std::str::from_utf8(&line.as_bytes()[line.find(' ').unwrap() + 1..]).unwrap();
                let line = std::str::from_utf8(&line.as_bytes()[line.find(' ').unwrap() + 1..]).unwrap();
                let index2 = line.find(' ').unwrap();
                let mut value = std::str::from_utf8(&line.as_bytes()[..index2]).unwrap().parse::<f64>().unwrap();
                let line = std::str::from_utf8(&line.as_bytes()[index2+1..]).unwrap();
                let index3 = line.find(' ').unwrap();
                let units = std::str::from_utf8(&line.as_bytes()[..index3]).unwrap();
                if units == "µs" {
                    // Do nothing.
                } else if units == "ms" {
                    value *= 1_000.0;
                } else {
                    panic!("Unknown units: {}", units);
                }

                if png_filename.contains("ztest") {
                    if let Some(val) = hashmap.get_mut(&png_filename) {
                        val.push(value.to_string());
                    } else {
                        hashmap.insert(png_filename.clone(), vec![png_filename.clone(), value.to_string()]);
                    };
                }
            },
        }
    }
    Ok(())
}

fn save_csv(path: &Path, hashmap: HashMap<String, Vec<String>>) -> Result<()> {
    let mut file = File::create(&path)?;
    let mut keys: Vec<String> = hashmap.keys().map(|s| s.clone()).collect();
    keys.sort();
    for key in keys.iter() {
        for entry in hashmap[key].iter() {
            write!(&mut file, "{},", entry)?;
        }
        writeln!(&mut file)?;
    }
    Ok(())
}

fn handle_file(arg: &str, entries: &mut Vec<String>) -> Result<()> {
    if arg == "!header" {
        return Ok(());
    }
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
    if let Some(bp) = reader.block_properties() {
        entries.push(bp.doubles_for_9_bit_table.to_string());
        entries.push(bp.doubles_for_12_bit_table.to_string());
        entries.push(bp.secondary_lookups_for_9_bit_table.to_string());
        entries.push(bp.secondary_lookups_for_12_bit_table.to_string());
    } else {
        entries.push("".to_string());
        entries.push("".to_string());
        entries.push("".to_string());
        entries.push("".to_string());
    }
    Ok(())
}

fn main() -> Result<()> {
    // Invoke with `path-to-csv.csv`.  The 1st column should be the filename
    // (it will be opened from '../tests/benches/' directory - see `handle_file` above).
    let mut hashmap = HashMap::new();
    hashmap.insert("!header".to_string(), vec!["filename".to_string()]);
    for arg in std::env::args().skip(1) {
        let arg = arg.to_string();
        let path = Path::new(&arg);
        load_csv(&mut hashmap, path)?;

        let mut len = None;
        for val in hashmap.values() {
            let len = *len.get_or_insert(val.len());
            assert_eq!(len, val.len());
        }
    }

    if let Some(val) = hashmap.get_mut("!header") {
        val.push("bits_per_sample".to_string());
        val.push("color_type".to_string());
        val.push("interlaced".to_string());
        val.push("frame_buffer_size".to_string());
        val.push("first_idat_size".to_string());
        val.push("filesize".to_string());
        val.push("doubles9".to_string());
        val.push("doubles12".to_string());
        val.push("secondaries9".to_string());
        val.push("secondaries12".to_string());
    } else {
        panic!("No header");
    };
    for (key, values) in hashmap.iter_mut() {
        handle_file(key, values)?;
    }
    save_csv(Path::new("ztest-stats.csv"), hashmap)?;
    Ok(())
}
