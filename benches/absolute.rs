use png::Decoder;
use std::hint::black_box;
use std::io::{stdout, Write};
use std::time::Instant;

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
    data: &'static [u8],
    repetitions: usize,
}

const TESTS: [Test; 6] = [
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
    Test {
        name: "Lohengrin_-_Illustrated_Sporting_and_Dramatic_News.png",
        data: include_bytes!(
            "../tests/benches/Lohengrin_-_Illustrated_Sporting_and_Dramatic_News.png"
        ),
        repetitions: 70,
    },
    Test {
        name: "Transparency.png",
        data: include_bytes!("../tests/benches/Transparency.png"),
        repetitions: 20000,
    },
];

fn main() {
    for test in TESTS.iter() {
        print!(
            "Starting {} repetitions of decoding {} ...",
            test.repetitions, test.name
        );
        stdout().flush().unwrap();
        let start = Instant::now();
        //for _ in 0..test.repetitions {
        bench_file_once(test.data);
        //}
        println!(" done in {}ms.", start.elapsed().as_millis());
    }
}
