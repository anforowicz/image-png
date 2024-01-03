use png::Decoder;
use std::hint::black_box;
use std::io::{stdout, Write};
use std::time::Instant;

#[path = "../src/test_utils.rs"]
mod test_utils;

fn bench_file_once(data: &[u8]) -> Vec<u8> {
    let data = black_box(data);
    black_box({
        let decoder = Decoder::new(&*data);
        let mut reader = decoder.read_info().unwrap();
        let mut image = vec![0; reader.output_buffer_size()];
        reader.next_frame(&mut image).unwrap();
        image
    })
}

struct Test {
    name: &'static str,
    data: Vec<u8>,
    repetitions: usize,
}

fn main() {
    let mut tests = vec![
/*
    Test {
        name: "kodim02.png",
        data: include_bytes!("../tests/benches/kodim02.png"),
        repetitions: 500,
    },
    Test {
        name: "kodim07.png",
        data: include_bytes!("../tests/benches/kodim07.png"),
        repetitions: 500,
    },
    Test {
        name: "kodim17.png",
        data: include_bytes!("../tests/benches/kodim17.png"),
        repetitions: 500,
    },
    Test {
        name: "kodim23.png",
        data: include_bytes!("../tests/benches/kodim23.png"),
        repetitions: 500,
    },
*/

/*
        Test {
            name: "top500-www-gov-br-tree-collapsed.png",
            data: include_bytes!("../tests/benches/top500-www-gov-br-tree-collapsed.png"),
            repetitions: 500000,
        },
        Test {
            name: "top500-ok-ru-new-green.png",
            data: include_bytes!("../tests/benches/top500-ok-ru-new-green.png"),
            repetitions: 200000,
        },
*/
    ];
    tests.push(Test {
        name: "generated-noncompressed-64k-idat/2048x2048.png",
        data: {
            let mut data = Vec::new();
            test_utils::write_noncompressed_png(&mut data, 2048, 65536);
            data
        },
        repetitions: 500,
    });
    for test in tests.iter() {
        print!("Starting {} repetitions of decoding {} ...", test.repetitions, test.name);
        stdout().flush().unwrap();
        let start = Instant::now();
        for _ in 0..test.repetitions {
            bench_file_once(test.data.as_slice());
        }
        println!(" done in {}ms.", start.elapsed().as_millis());
    }
}
