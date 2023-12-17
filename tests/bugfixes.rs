use std::fs::File;
use std::io::BufReader;

use png::{DecodeOptions, Decoder, DecodingError};

#[test]
fn issue_430() {
    let file = File::open("tests/bugfixes/issue#430.png").unwrap();
    let input = BufReader::new(file);

    let mut decode_options = DecodeOptions::default();
    decode_options.set_skip_ancillary_crc_failures(false);

    let decoder = Decoder::new_with_options(input, decode_options).read_info();

    assert!(
        matches!(decoder, Err(DecodingError::Format(_))),
        "Decoding of iCCP chunk with invalid CRC should have failed with 'skip_ancillary_crc' disabled."
    );
}
