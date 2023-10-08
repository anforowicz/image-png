use super::{stream::FormatErrorInner, DecodingError};
use miniz_oxide::inflate::TINFLStatus;
use miniz_oxide::inflate::core::{DecompressorOxide, inflate_flags};

/// How far back inflate may need to look.
const LOOKBACK_SIZE: usize = 32 * 1024;

/// Capacity of `ZlibStream::in_buffer`.
///
/// This buffer should be needed fairly infrequently (only when `fdeflate` can't make progress due
/// to insufficient input - typically when crossing from one IDAT chunk to another).
const IN_BUFFER_SIZE: usize = 1024;

/// Capacity of `ZlibStream::out_buffer`.
///
/// Big capacity => `prepare_vec_for_appending` can compact less frequently, which means less
/// L1 cache trashing.
const OUT_BUFFER_SIZE: usize = 1024 * 1024;

/// Ergonomics wrapper around `miniz_oxide::inflate::stream` for zlib compressed data.
pub(super) struct ZlibStream {
    /// Current decoding state.
    state: Box<DecompressorOxide>,
    /// If there has been a call to decompress already.
    started: bool,
    /// A buffer of compressed data.
    /// We use this for a progress guarantee. The data in the input stream is chunked as given by
    /// the underlying stream buffer. We will not read any more data until the current buffer has
    /// been fully consumed. The zlib decompression can not fully consume all the data when it is
    /// in the middle of the stream, it will treat full symbols and maybe the last bytes need to be
    /// treated in a special way. The exact reason isn't as important but the interface does not
    /// promise us this. Now, the complication is that the _current_ chunking information of PNG
    /// alone is not enough to determine this as indeed the compressed stream is the concatenation
    /// of all consecutive `IDAT`/`fdAT` chunks. We would need to inspect the next chunk header.
    ///
    /// Thus, there needs to be a buffer that allows fully clearing a chunk so that the next chunk
    /// type can be inspected.
    in_buffer: Vec<u8>,
    /// The logical start of the `in_buffer`.
    in_pos: usize,
    /// Remaining buffered decoded bytes.
    /// The decoder sometimes wants inspect some already finished bytes for further decoding. So we
    /// keep a total of 32KB of decoded data available as long as more data may be appended.
    out_buffer: Vec<u8>,
    /// Index into `out_buffer` - points at the 1st decompressed byte that hasn't been `read` yet.
    reader_pos: usize,
    /// Index into `out_buffer` - 1 byte past the already decompressed data (i.e. pointing at
    /// where newly decompressed data may be written).
    out_pos: usize,
    /// Ignore and do not calculate the Adler-32 checksum. Defaults to `true`.
    ///
    /// This flag overrides `TINFL_FLAG_COMPUTE_ADLER32`.
    ///
    /// This flag should not be modified after decompression has started.
    ignore_adler32: bool,
    /// How many bytes we predict our client, reader will need next.
    predicted_read_size: usize,
}

impl ZlibStream {
    pub(crate) fn new() -> Self {
        ZlibStream {
            state: Box::new(DecompressorOxide::new()),
            started: false,
            in_buffer: Vec::with_capacity(IN_BUFFER_SIZE),
            in_pos: 0,
            out_buffer: Vec::with_capacity(OUT_BUFFER_SIZE),
            reader_pos: 0,
            out_pos: 0,
            ignore_adler32: true,
            predicted_read_size: 4096,
        }
    }

    pub(crate) fn reset(&mut self, rowlen: usize) {
        self.started = false;
        self.in_buffer.clear();
        self.in_pos = 0;
        self.out_buffer.clear();
        self.reader_pos = 0;
        self.out_pos = 0;
        *self.state = DecompressorOxide::new();

        // In an attempt to fit our whole working set into L1 cache, we try to decompress only as
        // much as needed to unfilter the next row of PNG data.  OTOH, for small `rowlen` this
        // might be excessively small, so we use a lower bound of 1024 bytes.
        //
        // We predict a slightly larger read size (by 8 bytes), because some decompression crates
        // (e.g. `fdeflate`) are not always able to decompress *exactly* the required number of
        // bytes.
        //
        // Note that initial rows in interlaced images will be smaller, but this is okay.
        self.predicted_read_size = std::cmp::max(1024, rowlen + 8);
    }

    /// Set the `ignore_adler32` flag and return `true` if the flag was
    /// successfully set.
    ///
    /// The default is `true`.
    ///
    /// This flag cannot be modified after decompression has started until the
    /// [ZlibStream] is reset.
    pub(crate) fn set_ignore_adler32(&mut self, flag: bool) -> bool {
        if !self.started {
            self.ignore_adler32 = flag;
            true
        } else {
            false
        }
    }

    /// Return the `ignore_adler32` flag.
    pub(crate) fn ignore_adler32(&self) -> bool {
        self.ignore_adler32
    }

    /// Fill the decoded buffer as far as possible from `data`.
    /// On success returns the number of consumed input bytes.
    pub(crate) fn decompress(&mut self, data: &[u8]) -> Result<usize, DecodingError> {
        self.prepare_vec_for_appending();

        let using_in_buffer = !self.in_buffer.is_empty();
        let in_data = if using_in_buffer {
            debug_assert!(self.in_pos < self.in_buffer.len());
            &self.in_buffer[self.in_pos..]
        } else {
            data
        };

        let (_status, in_consumed, out_consumed) = miniz_oxide::inflate::core::decompress(
            &mut self.state,
            in_data,
            self.out_buffer.as_mut_slice(),
            self.out_pos,
            inflate_flags::TINFL_FLAG_USING_NON_WRAPPING_OUTPUT_BUF |
            inflate_flags::TINFL_FLAG_HAS_MORE_INPUT |
            inflate_flags::TINFL_FLAG_PARSE_ZLIB_HEADER |
            inflate_flags::TINFL_FLAG_IGNORE_ADLER32 // DO NOT SUBMIT?,
        );
        // TODO / DO NOT SUBMIT - map `status`
        //    .map_err(|err| {
        //        DecodingError::Format(FormatErrorInner::CorruptFlateStream { err }.into())
        //    })?;
        self.started = true;
        self.out_pos += out_consumed;
        if using_in_buffer {
            self.in_pos += in_consumed;
            debug_assert!(self.in_pos <= self.in_buffer.len());
            if self.in_pos == self.in_buffer.len() {
                self.in_buffer.clear();
                self.in_pos = 0;
            }
        }

        let made_progress = in_consumed != 0 || out_consumed != 0;
        let data_consumed = if made_progress {
            if using_in_buffer {
                0
            } else {
                in_consumed
            }
        } else {
            // Compact `in_buffer` before extending it.
            if !self.in_buffer.is_empty() {
                let new_in_buffer_len = self.in_buffer.len() - self.in_pos;
                self.in_buffer.copy_within(self.in_pos.., 0);
                self.in_buffer.truncate(new_in_buffer_len);
                self.in_pos = 0;
            }

            // Append all or part of `data` to `in_buffer`.
            let in_buffer_space_left = self.in_buffer.capacity() - self.in_buffer.len();
            let data_consumed = std::cmp::min(in_buffer_space_left, data.len());
            self.in_buffer.extend_from_slice(&data[..data_consumed]);

            // Double-check that `in_buffer` doesn't grow - out L1-cache-friendliness depends on it
            debug_assert_eq!(self.in_buffer.capacity(), IN_BUFFER_SIZE);

            data_consumed
        };
        debug_assert!(data_consumed <= data.len());
        Ok(data_consumed)
    }

    /// Called after all consecutive IDAT chunks were handled.
    ///
    /// The compressed stream can be split on arbitrary byte boundaries. This enables some cleanup
    /// within the decompressor and flushing additional data which may have been kept back in case
    /// more data were passed to it.
    pub(crate) fn finish_compressed_chunks(&mut self) -> Result<(), DecodingError> {
        if !self.started {
            return Ok(());
        }

        loop {
            self.prepare_vec_for_appending();

            let (status, in_consumed, out_consumed) = miniz_oxide::inflate::core::decompress(
                &mut self.state,
                &self.in_buffer[self.in_pos..],
                self.out_buffer.as_mut_slice(),
                self.out_pos,
                inflate_flags::TINFL_FLAG_USING_NON_WRAPPING_OUTPUT_BUF |
                inflate_flags::TINFL_FLAG_PARSE_ZLIB_HEADER |
                inflate_flags::TINFL_FLAG_IGNORE_ADLER32 // DO NOT SUBMIT?,
            );
            // TODO / DO NOT SUBMIT - handle `status`
            //        .map_err(|err| {
            //            DecodingError::Format(FormatErrorInner::CorruptFlateStream { err }.into())
            //        })?;

            self.in_pos += in_consumed;
            self.out_pos += out_consumed;

            if status == TINFLStatus::Done {
                return Ok(());
            } else if in_consumed == 0 && out_consumed == 0 {
                return Err(DecodingError::Format(
                    FormatErrorInner::CorruptFlateStream {
                        err: fdeflate::DecompressionError::InsufficientInput,
                    }
                    .into(),
                ));
            }
        }
    }

    /// Ensure that there are at least `self.predicted_read_size` bytes in
    /// `self.out_buffer[self.out_pos..]`.
    fn prepare_vec_for_appending(&mut self) {
        let mut target_len = std::cmp::max(
            self.reader_pos.saturating_add(self.predicted_read_size),
            self.out_pos + 2, // Minimum room required by `fdeflate`.
        );
        if target_len <= self.out_buffer.len() {
            return;
        }

        // Compact `self.out_buffer` if needed.
        //
        // The `Vec::resize` call below may require a bigger buffer than the vector's current
        // capacity.  Copying all of `Vec`'s data into the new, bigger buffer would result in
        // undesirable trashing of L1 cache.  Therefore we try to avoid this by compacting
        // `self.out_buffer` instead.
        //
        // Compacting the buffer moves `self.out_buffer[safe..self.out_pos` bytes to the beginning
        // of the buffer (discarding the bytes in `self.out_buffer[..safe]`).  This shortens the
        // current buffer and should avoid the need to grow/reallocate `Vec`'s buffer.  Compacting
        // still has to copy *some* bytes, but typically `self.out_pos - self.safe_pos` is much
        // smaller than `out_buffer.len()`.  Compacting also requires re-zeroing bytes in
        // `self.out_buffer[self.out_pos..], but again the cost is smaller than the cost of growing
        // the `Vec` (one factor is that `memset` is cheaper than `memcpy`).
        //
        // Still, compacting has non-zero cost (it will also trash L1 cache, because it has to at
        // least move 32kB / `LOOKBACK_SIZE` bytes), so we try to do it infrequently by using
        // `self.out_buffer` with a huge capacity (see `OUT_BUFFER_SIZE`).
        if target_len > self.out_buffer.capacity() {
            // Only discard "safe" bytes: ones that have already been read and are not needed for
            // inflate algorithm look-back.
            let safe = std::cmp::min(self.reader_pos, self.out_pos.saturating_sub(LOOKBACK_SIZE));

            // Move everything to the left.
            self.out_buffer.copy_within(safe..self.out_pos, 0);
            self.out_pos -= safe;
            self.reader_pos -= safe;
            target_len -= safe;

            // `truncate` helps to ensure that `resize` below will zero-out bytes beyond `out_pos`.
            self.out_buffer.truncate(self.out_pos);
        }

        self.out_buffer.resize(target_len, 0);

        // To be L1-cache-friendly we shouldn't grow/reallocate `out_buffer` - it should stay at
        // the same capacity throughout its lifetime.
        debug_assert_eq!(OUT_BUFFER_SIZE, self.out_buffer.capacity());
    }

    /// Returns the decompressed output.  This is an alternative to `self.read_to_end` that doesn't
    /// require copying of the data.
    pub(crate) fn into_vec(mut self) -> Vec<u8> {
        if self.reader_pos != 0 {
            self.out_buffer
                .copy_within(self.reader_pos..self.out_pos, 0);
            self.out_pos -= self.reader_pos;
        }
        self.out_buffer.truncate(self.out_pos);
        self.out_buffer
    }
}

impl std::io::Read for ZlibStream {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        static mut N_COUNT: usize = 0;
        static mut MISS_COUNT: usize = 0;
        super::cache_counters::start();

        let n = std::cmp::min(buf.len(), self.out_pos - self.reader_pos);
        buf[0..n].copy_from_slice(&self.out_buffer[self.reader_pos..self.reader_pos + n]);
        self.reader_pos += n;

        let (n_count, miss_count): (usize, usize) = {
            let cache_misses = super::cache_counters::stop();
            unsafe {
                N_COUNT += n;
                MISS_COUNT += cache_misses as usize;
                (N_COUNT, MISS_COUNT)
            }
        };
        println!(
            "miss_count={miss_count}, n_count={n_count}, n/miss={}",
            n_count / miss_count
        );

        Ok(n)
    }
}
