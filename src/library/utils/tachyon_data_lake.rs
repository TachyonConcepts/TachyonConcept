use std::arch::x86_64::{
    __m128i, __m256i, _mm256_loadu_si256, _mm256_storeu_si256, _mm_loadu_si128, _mm_storeu_si128,
};

pub struct TachyonDataLakeTools;

impl TachyonDataLakeTools {
    #[inline(always)]
    pub unsafe fn write_to(mut dst: *mut u8, mut src: *const u8, mut len: usize) {
        while len >= 32 {
            let v: __m256i = _mm256_loadu_si256(src as *const __m256i);
            _mm256_storeu_si256(dst as *mut __m256i, v);
            src = src.add(32);
            dst = dst.add(32);
            len -= 32;
        }
        while len >= 16 {
            let v: __m128i = _mm_loadu_si128(src as *const __m128i);
            _mm_storeu_si128(dst as *mut __m128i, v);
            src = src.add(16);
            dst = dst.add(16);
            len -= 16;
        }
        while len >= 8 {
            let val: u64 = core::ptr::read_unaligned(src as *const u64);
            core::ptr::write_unaligned(dst as *mut u64, val);
            src = src.add(8);
            dst = dst.add(8);
            len -= 8;
        }
        if len >= 4 {
            let val: u32 = core::ptr::read_unaligned(src as *const u32);
            core::ptr::write_unaligned(dst as *mut u32, val);
            src = src.add(4);
            dst = dst.add(4);
            len -= 4;
        }
        while len > 0 {
            *dst = *src;
            src = src.add(1);
            dst = dst.add(1);
            len -= 1;
        }
    }
}

#[repr(C)]
#[derive(Clone)]
pub struct TachyonDataLake<const N: usize> {
    pub(super) buf: [u8; N],
    pub(super) pos: usize,
}

impl<const N: usize> TachyonDataLake<N> {
    #[inline(always)]
    pub const fn build() -> Self {
        Self {
            buf: [0u8; N],
            pos: 0,
        }
    }
    #[inline(always)]
    pub fn reset_pos(&mut self) {
        self.pos = 0;
    }
    #[inline(always)]
    pub unsafe fn write_byte(&mut self, c: u8) {
        *self.buf.as_mut_ptr().add(self.pos) = c;
        self.pos += 1;
        // Ring behavior
        if self.pos >= self.buf.len() {
            self.pos = 0;
        }
    }
    #[inline(always)]
    pub fn freeze_ref(&mut self) -> &Self {
        &*self
    }
    #[inline(always)]
    pub fn freeze_ptr(&self) -> *const Self {
        self as *const Self
    }
    #[inline(always)]
    pub unsafe fn as_slice(&self) -> &[u8] {
        &self.buf[..self.pos]
    }
    #[inline(always)]
    pub fn len(&self) -> usize {
        self.pos
    }
    #[inline(always)]
    pub unsafe fn as_ptr(&self) -> *const u8 {
        self.buf.as_ptr()
    }
    #[inline(always)]
    pub unsafe fn as_mut_ptr(&mut self) -> *mut u8 {
        self.buf.as_mut_ptr().add(self.pos)
    }
    #[inline(always)]
    pub unsafe fn write(&mut self, src: *const u8, len: usize) {
        let buf_len: usize = self.buf.len();
        if len > buf_len {
            panic!("DataLake overflow: trying to write {len}, but buffer is only {buf_len}");
        }
        let dst: *mut u8 = if len <= (buf_len - self.pos) {
            self.buf.as_mut_ptr().add(self.pos)
        } else {
            self.pos = 0;
            self.buf.as_mut_ptr()
        };
        TachyonDataLakeTools::write_to(dst, src, len);
        self.pos += len;
    }
    #[inline(always)]
    pub unsafe fn write_num_str(&mut self, mut value: usize) {
        let mut tmp: [u8; 20] = [0u8; 20];
        let mut curr = tmp.len();

        while value >= 10 {
            let rem = value % 10;
            value /= 10;
            curr -= 1;
            *tmp.get_unchecked_mut(curr) = (rem as u8) + b'0';
        }
        curr -= 1;
        *tmp.get_unchecked_mut(curr) = (value as u8) + b'0';

        let len = tmp.len() - curr;
        self.write(tmp.as_ptr().add(curr), len);
    }
    #[inline(always)]
    pub unsafe fn write_num_str_fixed(&mut self, mut value: usize, len: usize) {
        let dst = self.buf.as_mut_ptr().add(self.pos + len);
        let mut ptr = dst;

        for _ in 0..len {
            ptr = ptr.offset(-1);
            *ptr = ((value % 10) as u8) + b'0';
            value /= 10;
        }

        self.pos += len;
    }
    #[allow(dead_code)]
    #[inline(always)]
    fn into_raw_parts(mut self) -> (*mut u8, usize) {
        let ptr = self.buf.as_mut_ptr();
        let len = self.pos;
        core::mem::forget(self);
        (ptr, len)
    }
}
