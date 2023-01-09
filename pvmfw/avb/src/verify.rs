// Copyright 2022, The Android Open Source Project
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! This module handles the pvmfw payload verification.

use avb_bindgen::{
    avb_slot_verify, AvbHashtreeErrorMode, AvbIOResult, AvbOps, AvbSlotVerifyFlags,
    AvbSlotVerifyResult,
};
use core::{
    ffi::{c_char, c_void, CStr},
    fmt,
    ptr::{self, NonNull},
    slice,
};

/// Error code from AVB image verification.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AvbImageVerifyError {
    /// AVB_SLOT_VERIFY_RESULT_ERROR_INVALID_ARGUMENT
    InvalidArgument,
    /// AVB_SLOT_VERIFY_RESULT_ERROR_INVALID_METADATA
    InvalidMetadata,
    /// AVB_SLOT_VERIFY_RESULT_ERROR_IO
    Io,
    /// AVB_SLOT_VERIFY_RESULT_ERROR_OOM
    Oom,
    /// AVB_SLOT_VERIFY_RESULT_ERROR_PUBLIC_KEY_REJECTED
    PublicKeyRejected,
    /// AVB_SLOT_VERIFY_RESULT_ERROR_ROLLBACK_INDEX
    RollbackIndex,
    /// AVB_SLOT_VERIFY_RESULT_ERROR_UNSUPPORTED_VERSION
    UnsupportedVersion,
    /// AVB_SLOT_VERIFY_RESULT_ERROR_VERIFICATION
    Verification,
}

impl fmt::Display for AvbImageVerifyError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Self::InvalidArgument => write!(f, "Invalid parameters."),
            Self::InvalidMetadata => write!(f, "Invalid metadata."),
            Self::Io => write!(f, "I/O error while trying to load data or get a rollback index."),
            Self::Oom => write!(f, "Unable to allocate memory."),
            Self::PublicKeyRejected => write!(f, "Public key rejected or data not signed."),
            Self::RollbackIndex => write!(f, "Rollback index is less than its stored value."),
            Self::UnsupportedVersion => write!(
                f,
                "Some of the metadata requires a newer version of libavb than what is in use."
            ),
            Self::Verification => write!(f, "Data does not verify."),
        }
    }
}

fn to_avb_verify_result(result: AvbSlotVerifyResult) -> Result<(), AvbImageVerifyError> {
    match result {
        AvbSlotVerifyResult::AVB_SLOT_VERIFY_RESULT_OK => Ok(()),
        AvbSlotVerifyResult::AVB_SLOT_VERIFY_RESULT_ERROR_INVALID_ARGUMENT => {
            Err(AvbImageVerifyError::InvalidArgument)
        }
        AvbSlotVerifyResult::AVB_SLOT_VERIFY_RESULT_ERROR_INVALID_METADATA => {
            Err(AvbImageVerifyError::InvalidMetadata)
        }
        AvbSlotVerifyResult::AVB_SLOT_VERIFY_RESULT_ERROR_IO => Err(AvbImageVerifyError::Io),
        AvbSlotVerifyResult::AVB_SLOT_VERIFY_RESULT_ERROR_OOM => Err(AvbImageVerifyError::Oom),
        AvbSlotVerifyResult::AVB_SLOT_VERIFY_RESULT_ERROR_PUBLIC_KEY_REJECTED => {
            Err(AvbImageVerifyError::PublicKeyRejected)
        }
        AvbSlotVerifyResult::AVB_SLOT_VERIFY_RESULT_ERROR_ROLLBACK_INDEX => {
            Err(AvbImageVerifyError::RollbackIndex)
        }
        AvbSlotVerifyResult::AVB_SLOT_VERIFY_RESULT_ERROR_UNSUPPORTED_VERSION => {
            Err(AvbImageVerifyError::UnsupportedVersion)
        }
        AvbSlotVerifyResult::AVB_SLOT_VERIFY_RESULT_ERROR_VERIFICATION => {
            Err(AvbImageVerifyError::Verification)
        }
    }
}

enum AvbIOError {
    /// AVB_IO_RESULT_ERROR_OOM,
    #[allow(dead_code)]
    Oom,
    /// AVB_IO_RESULT_ERROR_IO,
    #[allow(dead_code)]
    Io,
    /// AVB_IO_RESULT_ERROR_NO_SUCH_PARTITION,
    NoSuchPartition,
    /// AVB_IO_RESULT_ERROR_RANGE_OUTSIDE_PARTITION,
    RangeOutsidePartition,
    /// AVB_IO_RESULT_ERROR_NO_SUCH_VALUE,
    NoSuchValue,
    /// AVB_IO_RESULT_ERROR_INVALID_VALUE_SIZE,
    InvalidValueSize,
    /// AVB_IO_RESULT_ERROR_INSUFFICIENT_SPACE,
    #[allow(dead_code)]
    InsufficientSpace,
}

impl From<AvbIOError> for AvbIOResult {
    fn from(error: AvbIOError) -> Self {
        match error {
            AvbIOError::Oom => AvbIOResult::AVB_IO_RESULT_ERROR_OOM,
            AvbIOError::Io => AvbIOResult::AVB_IO_RESULT_ERROR_IO,
            AvbIOError::NoSuchPartition => AvbIOResult::AVB_IO_RESULT_ERROR_NO_SUCH_PARTITION,
            AvbIOError::RangeOutsidePartition => {
                AvbIOResult::AVB_IO_RESULT_ERROR_RANGE_OUTSIDE_PARTITION
            }
            AvbIOError::NoSuchValue => AvbIOResult::AVB_IO_RESULT_ERROR_NO_SUCH_VALUE,
            AvbIOError::InvalidValueSize => AvbIOResult::AVB_IO_RESULT_ERROR_INVALID_VALUE_SIZE,
            AvbIOError::InsufficientSpace => AvbIOResult::AVB_IO_RESULT_ERROR_INSUFFICIENT_SPACE,
        }
    }
}

fn to_avb_io_result(result: Result<(), AvbIOError>) -> AvbIOResult {
    result.map_or_else(|e| e.into(), |_| AvbIOResult::AVB_IO_RESULT_OK)
}

extern "C" fn read_is_device_unlocked(
    _ops: *mut AvbOps,
    out_is_unlocked: *mut bool,
) -> AvbIOResult {
    if let Err(e) = is_not_null(out_is_unlocked) {
        return e.into();
    }
    // SAFETY: It is safe as the raw pointer `out_is_unlocked` is a valid pointer.
    unsafe {
        *out_is_unlocked = false;
    }
    AvbIOResult::AVB_IO_RESULT_OK
}

extern "C" fn read_from_partition(
    ops: *mut AvbOps,
    partition: *const c_char,
    offset: i64,
    num_bytes: usize,
    buffer: *mut c_void,
    out_num_read: *mut usize,
) -> AvbIOResult {
    to_avb_io_result(try_read_from_partition(
        ops,
        partition,
        offset,
        num_bytes,
        buffer,
        out_num_read,
    ))
}

fn try_read_from_partition(
    ops: *mut AvbOps,
    partition: *const c_char,
    offset: i64,
    num_bytes: usize,
    buffer: *mut c_void,
    out_num_read: *mut usize,
) -> Result<(), AvbIOError> {
    let ops = as_avbops_ref(ops)?;
    let partition = ops.as_ref().get_partition(partition)?;
    let buffer = to_nonnull(buffer)?;
    // SAFETY: It is safe to copy the requested number of bytes to `buffer` as `buffer`
    // is created to point to the `num_bytes` of bytes in memory.
    let buffer_slice = unsafe { slice::from_raw_parts_mut(buffer.as_ptr() as *mut u8, num_bytes) };
    copy_data_to_dst(partition, offset, buffer_slice)?;
    let out_num_read = to_nonnull(out_num_read)?;
    // SAFETY: The raw pointer `out_num_read` was created to point to a valid a `usize`
    // and we checked it is nonnull.
    unsafe {
        *out_num_read.as_ptr() = buffer_slice.len();
    }
    Ok(())
}

fn copy_data_to_dst(src: &[u8], offset: i64, dst: &mut [u8]) -> Result<(), AvbIOError> {
    let start = to_copy_start(offset, src.len()).ok_or(AvbIOError::InvalidValueSize)?;
    let end = start.checked_add(dst.len()).ok_or(AvbIOError::InvalidValueSize)?;
    dst.copy_from_slice(src.get(start..end).ok_or(AvbIOError::RangeOutsidePartition)?);
    Ok(())
}

fn to_copy_start(offset: i64, len: usize) -> Option<usize> {
    usize::try_from(offset)
        .ok()
        .or_else(|| isize::try_from(offset).ok().and_then(|v| len.checked_add_signed(v)))
}

extern "C" fn get_size_of_partition(
    ops: *mut AvbOps,
    partition: *const c_char,
    out_size_num_bytes: *mut u64,
) -> AvbIOResult {
    to_avb_io_result(try_get_size_of_partition(ops, partition, out_size_num_bytes))
}

fn try_get_size_of_partition(
    ops: *mut AvbOps,
    partition: *const c_char,
    out_size_num_bytes: *mut u64,
) -> Result<(), AvbIOError> {
    let ops = as_avbops_ref(ops)?;
    let partition = ops.as_ref().get_partition(partition)?;
    let partition_size =
        u64::try_from(partition.len()).map_err(|_| AvbIOError::InvalidValueSize)?;
    let out_size_num_bytes = to_nonnull(out_size_num_bytes)?;
    // SAFETY: The raw pointer `out_size_num_bytes` was created to point to a valid a `u64`
    // and we checked it is nonnull.
    unsafe {
        *out_size_num_bytes.as_ptr() = partition_size;
    }
    Ok(())
}

extern "C" fn read_rollback_index(
    _ops: *mut AvbOps,
    _rollback_index_location: usize,
    _out_rollback_index: *mut u64,
) -> AvbIOResult {
    // Rollback protection is not yet implemented, but
    // this method is required by `avb_slot_verify()`.
    AvbIOResult::AVB_IO_RESULT_OK
}

extern "C" fn get_unique_guid_for_partition(
    _ops: *mut AvbOps,
    _partition: *const c_char,
    _guid_buf: *mut c_char,
    _guid_buf_size: usize,
) -> AvbIOResult {
    // This method is required by `avb_slot_verify()`.
    AvbIOResult::AVB_IO_RESULT_OK
}

extern "C" fn validate_public_key_for_partition(
    ops: *mut AvbOps,
    partition: *const c_char,
    public_key_data: *const u8,
    public_key_length: usize,
    public_key_metadata: *const u8,
    public_key_metadata_length: usize,
    out_is_trusted: *mut bool,
    out_rollback_index_location: *mut u32,
) -> AvbIOResult {
    to_avb_io_result(try_validate_public_key_for_partition(
        ops,
        partition,
        public_key_data,
        public_key_length,
        public_key_metadata,
        public_key_metadata_length,
        out_is_trusted,
        out_rollback_index_location,
    ))
}

#[allow(clippy::too_many_arguments)]
fn try_validate_public_key_for_partition(
    ops: *mut AvbOps,
    partition: *const c_char,
    public_key_data: *const u8,
    public_key_length: usize,
    _public_key_metadata: *const u8,
    _public_key_metadata_length: usize,
    out_is_trusted: *mut bool,
    _out_rollback_index_location: *mut u32,
) -> Result<(), AvbIOError> {
    is_not_null(public_key_data)?;
    // SAFETY: It is safe to create a slice with the given pointer and length as
    // `public_key_data` is a valid pointer and it points to an array of length
    // `public_key_length`.
    let public_key = unsafe { slice::from_raw_parts(public_key_data, public_key_length) };
    let ops = as_avbops_ref(ops)?;
    // Verifies the public key for the known partitions only.
    ops.as_ref().get_partition(partition)?;
    let trusted_public_key = ops.as_ref().trusted_public_key;
    let out_is_trusted = to_nonnull(out_is_trusted)?;
    // SAFETY: It is safe as the raw pointer `out_is_trusted` is a nonnull pointer.
    unsafe {
        *out_is_trusted.as_ptr() = public_key == trusted_public_key;
    }
    Ok(())
}

fn as_avbops_ref<'a>(ops: *mut AvbOps) -> Result<&'a AvbOps, AvbIOError> {
    let ops = to_nonnull(ops)?;
    // SAFETY: It is safe as the raw pointer `ops` is a nonnull pointer.
    unsafe { Ok(ops.as_ref()) }
}

fn to_nonnull<T>(p: *mut T) -> Result<NonNull<T>, AvbIOError> {
    NonNull::new(p).ok_or(AvbIOError::NoSuchValue)
}

fn is_not_null<T>(ptr: *const T) -> Result<(), AvbIOError> {
    if ptr.is_null() {
        Err(AvbIOError::NoSuchValue)
    } else {
        Ok(())
    }
}

struct Payload<'a> {
    kernel: &'a [u8],
    trusted_public_key: &'a [u8],
}

impl<'a> AsRef<Payload<'a>> for AvbOps {
    fn as_ref(&self) -> &Payload<'a> {
        let payload = self.user_data as *const Payload;
        // SAFETY: It is safe to cast the `AvbOps.user_data` to Payload as we have saved a
        // pointer to a valid value of Payload in user_data when creating AvbOps, and
        // assume that the Payload isn't used beyond the lifetime of the AvbOps that it
        // belongs to.
        unsafe { &*payload }
    }
}

impl<'a> Payload<'a> {
    const KERNEL_PARTITION_NAME: &[u8] = b"bootloader\0";

    fn kernel_partition_name(&self) -> &CStr {
        CStr::from_bytes_with_nul(Self::KERNEL_PARTITION_NAME).unwrap()
    }

    fn get_partition(&self, partition_name: *const c_char) -> Result<&[u8], AvbIOError> {
        is_not_null(partition_name)?;
        // SAFETY: It is safe as the raw pointer `partition_name` is a nonnull pointer.
        let partition_name = unsafe { CStr::from_ptr(partition_name) };
        match partition_name.to_bytes_with_nul() {
            Self::KERNEL_PARTITION_NAME => Ok(self.kernel),
            _ => Err(AvbIOError::NoSuchPartition),
        }
    }
}

/// Verifies the payload (signed kernel + initrd) against the trusted public key.
pub fn verify_payload(kernel: &[u8], trusted_public_key: &[u8]) -> Result<(), AvbImageVerifyError> {
    let mut payload = Payload { kernel, trusted_public_key };
    let mut avb_ops = AvbOps {
        user_data: &mut payload as *mut _ as *mut c_void,
        ab_ops: ptr::null_mut(),
        atx_ops: ptr::null_mut(),
        read_from_partition: Some(read_from_partition),
        get_preloaded_partition: None,
        write_to_partition: None,
        validate_vbmeta_public_key: None,
        read_rollback_index: Some(read_rollback_index),
        write_rollback_index: None,
        read_is_device_unlocked: Some(read_is_device_unlocked),
        get_unique_guid_for_partition: Some(get_unique_guid_for_partition),
        get_size_of_partition: Some(get_size_of_partition),
        read_persistent_value: None,
        write_persistent_value: None,
        validate_public_key_for_partition: Some(validate_public_key_for_partition),
    };
    // NULL is needed to mark the end of the array.
    let requested_partitions: [*const c_char; 2] =
        [payload.kernel_partition_name().as_ptr(), ptr::null()];
    let ab_suffix = CStr::from_bytes_with_nul(b"\0").unwrap();

    // SAFETY: It is safe to call `avb_slot_verify()` as the pointer arguments (`ops`,
    // `requested_partitions` and `ab_suffix`) passed to the method are all valid and
    // initialized. The last argument `out_data` is allowed to be null so that nothing
    // will be written to it.
    let result = unsafe {
        avb_slot_verify(
            &mut avb_ops,
            requested_partitions.as_ptr(),
            ab_suffix.as_ptr(),
            AvbSlotVerifyFlags::AVB_SLOT_VERIFY_FLAGS_NO_VBMETA_PARTITION,
            AvbHashtreeErrorMode::AVB_HASHTREE_ERROR_MODE_RESTART_AND_INVALIDATE,
            /*out_data=*/ ptr::null_mut(),
        )
    };
    to_avb_verify_result(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;
    use avb_bindgen::AvbFooter;
    use std::{fs, mem::size_of};

    const PUBLIC_KEY_RSA2048_PATH: &str = "data/testkey_rsa2048_pub.bin";
    const PUBLIC_KEY_RSA4096_PATH: &str = "data/testkey_rsa4096_pub.bin";
    const RANDOM_FOOTER_POS: usize = 30;

    /// This test uses the Microdroid payload compiled on the fly to check that
    /// the latest payload can be verified successfully.
    #[test]
    fn latest_valid_payload_is_verified_successfully() -> Result<()> {
        let kernel = load_latest_signed_kernel()?;
        let public_key = fs::read(PUBLIC_KEY_RSA4096_PATH)?;

        assert_eq!(Ok(()), verify_payload(&kernel, &public_key));
        Ok(())
    }

    #[test]
    fn payload_with_empty_public_key_fails_verification() -> Result<()> {
        assert_payload_verification_fails(
            &load_latest_signed_kernel()?,
            /*trusted_public_key=*/ &[0u8; 0],
            AvbImageVerifyError::PublicKeyRejected,
        )
    }

    #[test]
    fn payload_with_an_invalid_public_key_fails_verification() -> Result<()> {
        assert_payload_verification_fails(
            &load_latest_signed_kernel()?,
            /*trusted_public_key=*/ &[0u8; 512],
            AvbImageVerifyError::PublicKeyRejected,
        )
    }

    #[test]
    fn payload_with_a_different_valid_public_key_fails_verification() -> Result<()> {
        assert_payload_verification_fails(
            &load_latest_signed_kernel()?,
            &fs::read(PUBLIC_KEY_RSA2048_PATH)?,
            AvbImageVerifyError::PublicKeyRejected,
        )
    }

    #[test]
    fn unsigned_kernel_fails_verification() -> Result<()> {
        assert_payload_verification_fails(
            &fs::read("unsigned_test.img")?,
            &fs::read(PUBLIC_KEY_RSA4096_PATH)?,
            AvbImageVerifyError::Io,
        )
    }

    #[test]
    fn tampered_kernel_fails_verification() -> Result<()> {
        let mut kernel = load_latest_signed_kernel()?;
        kernel[1] = !kernel[1]; // Flip the bits

        assert_payload_verification_fails(
            &kernel,
            &fs::read(PUBLIC_KEY_RSA4096_PATH)?,
            AvbImageVerifyError::Verification,
        )
    }

    #[test]
    fn tampered_kernel_footer_fails_verification() -> Result<()> {
        let mut kernel = load_latest_signed_kernel()?;
        let avb_footer_index = kernel.len() - size_of::<AvbFooter>() + RANDOM_FOOTER_POS;
        kernel[avb_footer_index] = !kernel[avb_footer_index];

        assert_payload_verification_fails(
            &kernel,
            &fs::read(PUBLIC_KEY_RSA4096_PATH)?,
            AvbImageVerifyError::InvalidMetadata,
        )
    }

    fn assert_payload_verification_fails(
        kernel: &[u8],
        trusted_public_key: &[u8],
        expected_error: AvbImageVerifyError,
    ) -> Result<()> {
        assert_eq!(Err(expected_error), verify_payload(kernel, trusted_public_key));
        Ok(())
    }

    fn load_latest_signed_kernel() -> Result<Vec<u8>> {
        Ok(fs::read("microdroid_kernel")?)
    }
}