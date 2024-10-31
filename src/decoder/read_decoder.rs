use super::stream::{
    DecodeOptions, Decoded, DecodingError, FormatErrorInner, StreamingDecoder, CHUNK_BUFFER_SIZE,
};
use super::Limits;

use std::io::{BufRead, BufReader, ErrorKind, Read};

use crate::chunk;
use crate::common::Info;

/// Helper for encapsulating reading input from `Read` and feeding it into a `StreamingDecoder`
/// while hiding low-level `Decoded` events and only exposing a few high-level reading operations
/// like:
///
/// * `read_header_info` - reading until `IHDR` chunk
/// * `read_until_image_data` - reading until `IDAT` / `fdAT` sequence
/// * `decode_image_data` - reading from `IDAT` / `fdAT` sequence into `Vec<u8>`
/// * `finish_decoding_image_data()` - discarding remaining data from `IDAT` / `fdAT` sequence
/// * `read_until_end_of_input()` - reading until `IEND` chunk
pub(crate) struct ReadDecoder<R: Read> {
    reader: BufReader<R>,
    decoder: StreamingDecoder,
}

impl<R: Read> ReadDecoder<R> {
    pub fn new(r: R) -> Self {
        Self {
            reader: BufReader::with_capacity(CHUNK_BUFFER_SIZE, r),
            decoder: StreamingDecoder::new(),
        }
    }

    pub fn with_options(r: R, options: DecodeOptions) -> Self {
        let mut decoder = StreamingDecoder::new_with_options(options);
        decoder.limits = Limits::default();

        Self {
            reader: BufReader::with_capacity(CHUNK_BUFFER_SIZE, r),
            decoder,
        }
    }

    pub fn set_limits(&mut self, limits: Limits) {
        self.decoder.limits = limits;
    }

    pub fn reserve_bytes(&mut self, bytes: usize) -> Result<(), DecodingError> {
        self.decoder.limits.reserve_bytes(bytes)
    }

    pub fn set_ignore_text_chunk(&mut self, ignore_text_chunk: bool) {
        self.decoder.set_ignore_text_chunk(ignore_text_chunk);
    }

    pub fn set_ignore_iccp_chunk(&mut self, ignore_iccp_chunk: bool) {
        self.decoder.set_ignore_iccp_chunk(ignore_iccp_chunk);
    }

    pub fn ignore_checksums(&mut self, ignore_checksums: bool) {
        self.decoder.set_ignore_adler32(ignore_checksums);
        self.decoder.set_ignore_crc(ignore_checksums);
    }

    /// Returns the next decoded chunk. If the chunk is an ImageData chunk, its contents are passed
    /// into `image_data_callback`.
    fn decode_next(
        &mut self,
        image_data_callback: Option<&mut dyn FnMut(&[u8])>,
    ) -> Result<Decoded, DecodingError> {
        let (consumed, result) = {
            let buf = self.reader.fill_buf()?;
            if buf.is_empty() {
                return Err(DecodingError::IoError(ErrorKind::UnexpectedEof.into()));
            }
            self.decoder.update2(buf, image_data_callback)?
        };
        self.reader.consume(consumed);
        Ok(result)
    }

    fn decode_next_without_image_data(&mut self) -> Result<Decoded, DecodingError> {
        self.decode_next(Some(&mut |_| panic!("Unexpected image data")))
    }

    fn decode_next_and_discard_image_data(&mut self) -> Result<Decoded, DecodingError> {
        self.decode_next(None)
    }

    /// Reads until the end of `IHDR` chunk.
    ///
    /// Prerequisite: None (idempotent).
    pub fn read_header_info(&mut self) -> Result<&Info<'static>, DecodingError> {
        while self.info().is_none() {
            if let Decoded::ImageEnd = self.decode_next_without_image_data()? {
                unreachable!()
            }
        }
        Ok(self.info().unwrap())
    }

    /// Reads until the start of the next `IDAT` or `fdAT` chunk.
    ///
    /// Prerequisite: **Not** within `IDAT` / `fdAT` chunk sequence.
    pub fn read_until_image_data(&mut self) -> Result<(), DecodingError> {
        loop {
            match self.decode_next_without_image_data()? {
                Decoded::ChunkBegin(_, chunk::IDAT) | Decoded::ChunkBegin(_, chunk::fdAT) => break,
                Decoded::ImageEnd => {
                    return Err(DecodingError::Format(
                        FormatErrorInner::MissingImageData.into(),
                    ))
                }
                // Ignore all other chunk events. Any other chunk may be between IDAT chunks, fdAT
                // chunks and their control chunks.
                _ => {}
            }
        }
        Ok(())
    }

    /// Reads image data (passing it to `image_data_callback`) and reports whether there may be
    /// additional data afterwards (i.e. if it is okay to call `decode_image_data` and/or
    /// `finish_decoding_image_data` again).
    ///
    /// Prerequisite: Input is currently positioned within `IDAT` / `fdAT` chunk sequence.
    pub fn decode_image_data(
        &mut self,
        image_data_callback: &mut dyn FnMut(&[u8]),
    ) -> Result<ImageDataCompletionStatus, DecodingError> {
        self.decode_image_data_impl(Some(image_data_callback))
    }

    fn decode_image_data_impl(
        &mut self,
        image_data_callback: Option<&mut dyn FnMut(&[u8])>,
    ) -> Result<ImageDataCompletionStatus, DecodingError> {
        match self.decode_next(image_data_callback)? {
            Decoded::ImageData => Ok(ImageDataCompletionStatus::ExpectingMoreData),
            Decoded::ImageDataFlushed => Ok(ImageDataCompletionStatus::Done),
            // Ignore other events that may happen within an `IDAT` / `fdAT` chunks sequence.
            Decoded::Nothing
            | Decoded::ChunkComplete(_, _)
            | Decoded::ChunkBegin(_, _)
            | Decoded::PartialChunk(_) => Ok(ImageDataCompletionStatus::ExpectingMoreData),
            // Other kinds of events shouldn't happen, unless we have been (incorrectly) called
            // when outside of a sequence of `IDAT` / `fdAT` chunks.
            unexpected => unreachable!("{:?}", unexpected),
        }
    }

    /// Consumes and discards the rest of an `IDAT` / `fdAT` chunk sequence.
    ///
    /// Prerequisite: Input is currently positioned within `IDAT` / `fdAT` chunk sequence.
    pub fn finish_decoding_image_data(&mut self) -> Result<(), DecodingError> {
        loop {
            if let ImageDataCompletionStatus::Done = self.decode_image_data_impl(None)? {
                return Ok(());
            }
        }
    }

    /// Reads until the `IEND` chunk.
    ///
    /// Prerequisite: `IEND` chunk hasn't been reached yet.
    pub fn read_until_end_of_input(&mut self) -> Result<(), DecodingError> {
        while !matches!(
            self.decode_next_and_discard_image_data()?,
            Decoded::ImageEnd
        ) {}
        Ok(())
    }

    pub fn info(&self) -> Option<&Info<'static>> {
        self.decoder.info.as_ref()
    }
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) enum ImageDataCompletionStatus {
    ExpectingMoreData,
    Done,
}
