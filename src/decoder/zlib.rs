use super::{stream::FormatErrorInner, DecodingError};

use fdeflate::Decompressor;

/// Ergonomics wrapper around `miniz_oxide::inflate::stream` for zlib compressed data.
pub(super) struct ZlibStream {
    /// Current decoding state.
    state: Box<fdeflate::Decompressor>,
    /// If there has been a call to decompress already.
    started: bool,
    /// Remaining buffered decoded bytes.
    /// The decoder sometimes wants inspect some already finished bytes for further decoding. So we
    /// keep a total of 32KB of decoded data available as long as more data may be appended.
    out_buffer: Vec<u8>,
    /// The first index of `out_buffer` where new data can be written.
    out_pos: usize,
    /// The first index of `out_buffer` that hasn't yet been passed to our client
    /// (i.e. not yet appended to the `image_data` parameter of `fn decompress` or `fn
    /// finish_compressed_chunks`).
    read_pos: usize,
    /// Limit on how many bytes can be decompressed in total.  This field is mostly used for
    /// performance optimizations (e.g. to avoid allocating and zeroing out large buffers when only
    /// a small image is being decoded).
    max_total_output: usize,
    /// Ignore and do not calculate the Adler-32 checksum. Defaults to `true`.
    ///
    /// This flag overrides `TINFL_FLAG_COMPUTE_ADLER32`.
    ///
    /// This flag should not be modified after decompression has started.
    ignore_adler32: bool,
}

impl ZlibStream {
    pub(crate) fn new() -> Self {
        ZlibStream {
            state: Box::new(Decompressor::new()),
            started: false,
            out_buffer: Vec::new(),
            out_pos: 0,
            read_pos: 0,
            max_total_output: usize::MAX,
            ignore_adler32: true,
        }
    }

    pub(crate) fn reset(&mut self) {
        self.started = false;
        self.out_buffer.clear();
        self.out_pos = 0;
        self.read_pos = 0;
        self.max_total_output = usize::MAX;
        *self.state = Decompressor::new();
    }

    pub(crate) fn set_max_total_output(&mut self, n: usize) {
        self.max_total_output = n;
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
    pub(crate) fn decompress(
        &mut self,
        data: &[u8],
        image_data: &mut Vec<u8>,
    ) -> Result<usize, DecodingError> {
        self.prepare_vec_for_appending();

        if !self.started && self.ignore_adler32 {
            self.state.ignore_adler32();
        }

        let (in_consumed, out_consumed) = self
            .state
            .read(data, self.out_buffer.as_mut_slice(), self.out_pos, false)
            .map_err(|err| {
                DecodingError::Format(FormatErrorInner::CorruptFlateStream { err }.into())
            })?;

        self.started = true;
        self.out_pos += out_consumed;
        self.transfer_finished_data(image_data);
        self.compact_out_buffer_if_needed();

        Ok(in_consumed)
    }

    /// Called after all consecutive IDAT chunks were handled.
    ///
    /// The compressed stream can be split on arbitrary byte boundaries. This enables some cleanup
    /// within the decompressor and flushing additional data which may have been kept back in case
    /// more data were passed to it.
    pub(crate) fn finish_compressed_chunks(
        &mut self,
        image_data: &mut Vec<u8>,
    ) -> Result<(), DecodingError> {
        if !self.started {
            return Ok(());
        }

        while !self.state.is_done() {
            self.prepare_vec_for_appending();
            let (_in_consumed, out_consumed) = self
                .state
                .read(&[], self.out_buffer.as_mut_slice(), self.out_pos, true)
                .map_err(|err| {
                    DecodingError::Format(FormatErrorInner::CorruptFlateStream { err }.into())
                })?;

            self.out_pos += out_consumed;

            if !self.state.is_done() {
                let transferred = self.transfer_finished_data(image_data);
                assert!(
                    transferred > 0 || out_consumed > 0,
                    "No more forward progress made in stream decoding."
                );
                self.compact_out_buffer_if_needed();
            }
        }

        self.transfer_finished_data(image_data);
        self.out_buffer.clear();
        Ok(())
    }

    /// Resize the vector to allow allocation of more data.
    fn prepare_vec_for_appending(&mut self) {
        // The `debug_assert` below explains why we can use `>=` instead of `>` in the condition
        // that compares `self.out_post >= self.max_total_output` in the next `if` statement.
        debug_assert!(!self.state.is_done());
        if self.out_pos >= self.max_total_output {
            // This can happen when the `max_total_output` was miscalculated (e.g.
            // because the `IHDR` chunk was malformed and didn't match the `IDAT` chunk).  In
            // this case, let's reset `self.max_total_output` before further calculations.
            self.max_total_output = usize::MAX;
        }

        // Decompress at most `MAX_INCREMENTAL_DECOMPRESSION_SIZE`.  This helps to ensure that the
        // decompressed data fits into the L1 cache.  This is set lower than the typical L1 cache
        // size of 32kB, because we want to account for the fact that the decompressed data will be
        // copied at least once and to account for memory pressure from other data structures.
        //
        // TODO: The size of 16384 bytes is an ad-hoc heuristic.  Consider using a different number.
        const MAX_INCREMENTAL_DECOMPRESSION_SIZE: usize = 16384;
        let desired_len = self
            .out_pos
            .saturating_add(MAX_INCREMENTAL_DECOMPRESSION_SIZE);

        // Allocate at most `self.max_total_output`.  This avoids unnecessarily allocating and
        // zeroing-out a bigger `out_buffer` than necessary.
        let desired_len = desired_len.min(self.max_total_output);

        // Ensure the allocation request is valid.
        // TODO: maximum allocation limits?
        const _: () = assert!((isize::max_value() as usize) < u64::max_value() as usize);
        let desired_len = desired_len.min(isize::max_value() as usize);

        if self.out_buffer.len() < desired_len {
            self.out_buffer.resize(desired_len, 0u8);
        }
    }

    fn transfer_finished_data(&mut self, image_data: &mut Vec<u8>) -> usize {
        let transferred = &self.out_buffer[self.read_pos..self.out_pos];
        image_data.extend_from_slice(transferred);
        self.read_pos = self.out_pos;
        transferred.len()
    }

    fn compact_out_buffer_if_needed(&mut self) {
        // [PNG spec](https://www.w3.org/TR/2003/REC-PNG-20031110/#10Compression) says that
        // "deflate/inflate compression with a sliding window (which is an upper bound on the
        // distances appearing in the deflate stream) of at most 32768 bytes".
        //
        // `fdeflate` requires that we keep this many most recently decompressed bytes in the
        // `out_buffer` - this allows referring back to them when handling "length and distance
        // codes" in the deflate stream).
        const LOOKBACK_SIZE: usize = 32768;

        // Compact `self.out_buffer` when "needed".  Doing this conditionally helps to put an upper
        // bound on the amortized cost of copying the data within `self.out_buffer`.
        //
        // TODO: The factor of 4 is an ad-hoc heuristic.  Consider measuring and using a different
        // factor.  (Early experiments seem to indicate that factor of 4 is faster than a factor of
        // 2 and 4 * `LOOKBACK_SIZE` seems like an acceptable memory trade-off.  Higher factors
        // result in higher memory usage, but the compaction cost is lower - factor of 4 means
        // that 1 byte gets copied during compaction for 3 decompressed bytes.)
        if self.out_pos > LOOKBACK_SIZE * 4 {
            // Only preserve the `lookback_buffer` and "throw away" the earlier prefix.
            let lookback_buffer = self.out_pos.saturating_sub(LOOKBACK_SIZE)..self.out_pos;
            let preserved_len = lookback_buffer.len();
            self.out_buffer.copy_within(lookback_buffer, 0);
            self.read_pos = preserved_len;
            self.out_pos = preserved_len;

            // We can't reuse the rest of `out_buffer` because it hasn't been zeroed-out (as
            // expected by `fdeflate`.  Because of this, we `truncate` the `out_buffer`.
            self.out_buffer.truncate(preserved_len);
        }
    }
}
