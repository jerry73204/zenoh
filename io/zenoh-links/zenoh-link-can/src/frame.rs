//
// Copyright (c) 2026 ZettaScale Technology
//
// This program and the accompanying materials are made available under the
// terms of the Eclipse Public License 2.0 which is available at
// http://www.eclipse.org/legal/epl-2.0, or the Apache License, Version 2.0
// which is available at https://www.apache.org/licenses/LICENSE-2.0.
//
// SPDX-License-Identifier: EPL-2.0 OR Apache-2.0
//
// Contributors:
//   ZettaScale Zenoh Team, <zenoh@zettascale.tech>
//

//! The CAN link wire format.
//!
//! This module deliberately contains no I/O and no platform types, so every
//! rule below is a unit test that runs anywhere, with no `vcan0` and no root.
//!
//! One CAN frame carries one zenoh datagram. CAN FD payload lengths are
//! quantised — the DLC encodes 0..=8, 12, 16, 20, 24, 32, 48, 64 and nothing
//! between — so a 40-byte datagram travels in a 48-byte frame and the receiver
//! cannot recover the true length from the frame alone. Byte 0 of every payload
//! is therefore the datagram length and bytes 1..=N are the datagram.
//!
//! The format is fixed by the zenoh-pico implementation it interoperates with;
//! see RFC-0080 §4.1 and RFC-0081 §2. It is not ours to change unilaterally.

use core::fmt;

use zenoh_protocol::transport::BatchSize;

/// Byte 0 of the frame payload carries the true datagram length.
pub(crate) const LEN_PREFIX: usize = 1;

/// `CANFD_MAX_DLEN`.
pub(crate) const FD_MAX_DLEN: usize = 64;
/// `CAN_MAX_DLEN`.
pub(crate) const CLASSIC_MAX_DLEN: usize = 8;

/// Usable datagram bytes once the length prefix is subtracted.
pub(crate) const FD_MTU: BatchSize = (FD_MAX_DLEN - LEN_PREFIX) as BatchSize; // 63
pub(crate) const CLASSIC_MTU: BatchSize = (CLASSIC_MAX_DLEN - LEN_PREFIX) as BatchSize; // 7

/// `sizeof(struct can_frame)` and `sizeof(struct canfd_frame)`. These are the
/// read/write sizes the kernel uses to tell the two frame kinds apart.
pub(crate) const CAN_MTU_WIRE: usize = 16;
pub(crate) const CANFD_MTU_WIRE: usize = 72;

/// `CANFD_BRS` — use the fast data phase.
pub(crate) const CANFD_BRS: u8 = 0x01;

pub(crate) const CAN_SFF_MASK: u32 = 0x0000_07FF;
pub(crate) const CAN_EFF_MASK: u32 = 0x1FFF_FFFF;

/// The representable CAN FD frame lengths above 8.
const FD_DLC_STEPS: [u8; 7] = [12, 16, 20, 24, 32, 48, 64];

/// `struct canfd_frame` from `<linux/can.h>`.
///
/// Declared here rather than taken from `libc` because `libc`'s version keeps
/// its reserved fields private, so it cannot be constructed. The layout is
/// asserted against `libc`'s in [`assert_layout_matches_libc`] on Linux.
///
/// In classic mode only the first [`CAN_MTU_WIRE`] bytes are written, and they
/// overlay `struct can_frame` exactly: `len` lands on `can_dlc`, `flags` — zero
/// in classic mode — lands on `__pad`, and `res1` lands on `len8_dlc`.
#[repr(C, align(8))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Frame {
    pub(crate) can_id: u32,
    pub(crate) len: u8,
    pub(crate) flags: u8,
    pub(crate) res0: u8,
    pub(crate) res1: u8,
    pub(crate) data: [u8; FD_MAX_DLEN],
}

impl Frame {
    pub(crate) const fn zeroed() -> Self {
        Frame {
            can_id: 0,
            len: 0,
            flags: 0,
            res0: 0,
            res1: 0,
            data: [0u8; FD_MAX_DLEN],
        }
    }

    /// The first `wire` bytes of the frame, as they go to `write(2)`.
    ///
    /// `Frame` is `repr(C)` with no interior padding — 4 + 1 + 1 + 1 + 1 + 64 is
    /// exactly 72, and 72 is a multiple of the 8-byte alignment — so every byte
    /// of the struct is an initialised field byte.
    pub(crate) fn as_wire_bytes(&self, wire: usize) -> &[u8] {
        debug_assert!(wire <= core::mem::size_of::<Frame>());
        // SAFETY: `Frame` is `repr(C)` and padding-free, so it is valid to read
        // as a byte slice, and `wire` is bounded by its size.
        unsafe { core::slice::from_raw_parts(self as *const Frame as *const u8, wire) }
    }
}

/// Refusal reasons for [`encode`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TxError {
    /// The datagram does not fit one frame. zenoh's transport fragments to the
    /// link MTU before the link is called, so this is a bug at the call site
    /// rather than an expected runtime condition.
    TooLarge { len: usize, mtu: BatchSize },
    /// Only 11-bit identifiers are expressible: the sender never sets
    /// `CAN_EFF_FLAG`, so a larger value would silently become a different
    /// identifier on the wire. See RFC-0081 §2.1.
    IdentifierTooWide { id: u32 },
}

impl fmt::Display for TxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TxError::TooLarge { len, mtu } => {
                write!(f, "datagram of {len} bytes exceeds the CAN link MTU of {mtu}")
            }
            TxError::IdentifierTooWide { id } => write!(
                f,
                "CAN identifier {id:#x} exceeds the 11-bit range (max {CAN_SFF_MASK:#x}); \
                 extended identifiers are not part of this wire format"
            ),
        }
    }
}

/// Why a received frame was not delivered to the transport.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RxDrop {
    /// Neither a `can_frame` nor a `canfd_frame`: a runt or an error frame.
    NotAFrame { nread: usize },
    /// Our own transmission, heard back on a loopback-enabled interface.
    OwnFrame,
    /// Outside the identifier band this bus reserves for zenoh.
    Filtered { sender: u32 },
    /// No room for even the length prefix.
    NoLengthByte,
    /// The length byte disagrees with the frame length.
    BadLength { declared: usize, available: usize },
    /// The transport's buffer cannot hold the datagram. Dropping beats handing
    /// back a truncated datagram that would deserialise as garbage.
    BufferTooSmall { needed: usize, have: usize },
}

impl fmt::Display for RxDrop {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RxDrop::NotAFrame { nread } => write!(f, "read of {nread} bytes is not a CAN frame"),
            RxDrop::OwnFrame => write!(f, "own transmission"),
            RxDrop::Filtered { sender } => write!(f, "identifier {sender:#x} outside the band"),
            RxDrop::NoLengthByte => write!(f, "frame carries no length byte"),
            RxDrop::BadLength {
                declared,
                available,
            } => write!(
                f,
                "length byte declares {declared} bytes but only {available} are present"
            ),
            RxDrop::BufferTooSmall { needed, have } => {
                write!(f, "datagram of {needed} bytes does not fit a {have}-byte buffer")
            }
        }
    }
}

/// The usable datagram size for a mode.
pub(crate) const fn mtu(fd_mode: bool) -> BatchSize {
    if fd_mode {
        FD_MTU
    } else {
        CLASSIC_MTU
    }
}

/// The number of bytes handed to `write(2)`, which is how the kernel tells a
/// classic frame from an FD one.
pub(crate) const fn wire_len(fd_mode: bool) -> usize {
    if fd_mode {
        CANFD_MTU_WIRE
    } else {
        CAN_MTU_WIRE
    }
}

/// Round a payload length up to the next representable CAN FD frame length.
///
/// Below 9 bytes every length is representable and the payload is used as is;
/// above it only the DLC steps exist.
pub(crate) fn fd_frame_len(payload: usize) -> u8 {
    if payload <= CLASSIC_MAX_DLEN {
        return payload as u8;
    }
    for step in FD_DLC_STEPS {
        if payload <= step as usize {
            return step;
        }
    }
    // Unreachable for any payload <= 64, which `encode` has already enforced.
    FD_MAX_DLEN as u8
}

/// Build the frame carrying `datagram`, and the number of bytes to write.
pub(crate) fn encode(
    id: u32,
    datagram: &[u8],
    fd_mode: bool,
) -> Result<(Frame, usize), TxError> {
    if id > CAN_SFF_MASK {
        return Err(TxError::IdentifierTooWide { id });
    }
    let mtu = mtu(fd_mode);
    if datagram.len() > mtu as usize {
        return Err(TxError::TooLarge {
            len: datagram.len(),
            mtu,
        });
    }

    let mut frame = Frame::zeroed();
    frame.can_id = id;
    frame.data[0] = datagram.len() as u8;
    frame.data[LEN_PREFIX..LEN_PREFIX + datagram.len()].copy_from_slice(datagram);

    let payload = datagram.len() + LEN_PREFIX;
    frame.len = if fd_mode {
        // The bit-rate switch is requested for every FD frame, not only the
        // long ones: the data phase is where the rate gain is.
        frame.flags = CANFD_BRS;
        fd_frame_len(payload)
    } else {
        payload as u8
    };

    Ok((frame, wire_len(fd_mode)))
}

/// Decide what a received frame is, and copy out the datagram if it is one.
///
/// The rules are applied in the same order as the zenoh-pico receiver, so the
/// two implementations drop the same frames for the same reasons.
pub(crate) fn decode(
    frame: &Frame,
    nread: usize,
    own_id: u32,
    filter_match: u32,
    filter_mask: u32,
    out: &mut [u8],
) -> Result<(usize, u32), RxDrop> {
    if nread != CANFD_MTU_WIRE && nread != CAN_MTU_WIRE {
        return Err(RxDrop::NotAFrame { nread });
    }

    let sender = frame.can_id & CAN_EFF_MASK;
    if sender == own_id {
        return Err(RxDrop::OwnFrame);
    }
    if filter_mask != 0 && (sender & filter_mask) != filter_match {
        return Err(RxDrop::Filtered { sender });
    }

    let frame_len = frame.len as usize;
    if frame_len < LEN_PREFIX {
        return Err(RxDrop::NoLengthByte);
    }

    let declared = frame.data[0] as usize;
    let available = frame_len - LEN_PREFIX;
    if declared > available {
        return Err(RxDrop::BadLength {
            declared,
            available,
        });
    }
    if declared > out.len() {
        return Err(RxDrop::BufferTooSmall {
            needed: declared,
            have: out.len(),
        });
    }

    out[..declared].copy_from_slice(&frame.data[LEN_PREFIX..LEN_PREFIX + declared]);
    Ok((declared, sender))
}

#[cfg(all(test, target_os = "linux"))]
#[test]
fn assert_layout_matches_libc() {
    assert_eq!(
        core::mem::size_of::<Frame>(),
        core::mem::size_of::<libc::canfd_frame>()
    );
    assert_eq!(
        core::mem::align_of::<Frame>(),
        core::mem::align_of::<libc::canfd_frame>()
    );
    assert_eq!(core::mem::size_of::<Frame>(), CANFD_MTU_WIRE);
    assert_eq!(core::mem::size_of::<libc::can_frame>(), CAN_MTU_WIRE);
    assert_eq!(libc::CANFD_BRS as u8, CANFD_BRS);
    assert_eq!(libc::CAN_SFF_MASK, CAN_SFF_MASK);
    assert_eq!(libc::CAN_EFF_MASK, CAN_EFF_MASK);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn datagram(len: usize) -> Vec<u8> {
        (0..len).map(|i| (i + 1) as u8).collect()
    }

    #[test]
    fn mtus_are_the_frame_size_less_the_prefix() {
        assert_eq!(FD_MTU, 63);
        assert_eq!(CLASSIC_MTU, 7);
    }

    /// Every DLC boundary, as phase-378 W0 requires. The left column is the
    /// datagram length; the right is the resulting `frame.len`, which includes
    /// the length prefix.
    #[test]
    fn fd_dlc_steps() {
        let expected: &[(usize, u8)] = &[
            (0, 1),
            (7, 8),
            // payload 9 is past the last contiguous length, so it rounds
            (8, 12),
            (11, 12),
            (12, 16),
            (15, 16),
            (16, 20),
            (19, 20),
            (20, 24),
            (23, 24),
            (24, 32),
            (31, 32),
            (32, 48),
            (47, 48),
            (48, 64),
            (62, 64),
            (63, 64),
        ];
        for (len, frame_len) in expected {
            let (frame, wire) = encode(0x100, &datagram(*len), true).unwrap();
            assert_eq!(frame.len, *frame_len, "datagram of {len} bytes");
            assert_eq!(wire, CANFD_MTU_WIRE);
        }
    }

    #[test]
    fn classic_frame_len_is_the_payload() {
        for len in 0..=CLASSIC_MTU as usize {
            let (frame, wire) = encode(0x100, &datagram(len), false).unwrap();
            assert_eq!(frame.len as usize, len + LEN_PREFIX);
            assert_eq!(wire, CAN_MTU_WIRE);
            assert_eq!(frame.flags, 0, "classic frames carry no BRS");
        }
    }

    #[test]
    fn brs_is_set_on_every_fd_frame_including_short_ones() {
        for len in [0usize, 1, 7, 8, 63] {
            let (frame, _) = encode(0x100, &datagram(len), true).unwrap();
            assert_eq!(frame.flags, CANFD_BRS, "datagram of {len} bytes");
        }
    }

    #[test]
    fn over_mtu_is_refused_in_both_modes() {
        assert_eq!(
            encode(0x100, &datagram(64), true),
            Err(TxError::TooLarge { len: 64, mtu: 63 })
        );
        assert_eq!(
            encode(0x100, &datagram(8), false),
            Err(TxError::TooLarge { len: 8, mtu: 7 })
        );
    }

    #[test]
    fn extended_identifiers_are_refused() {
        assert_eq!(
            encode(0x800, &datagram(1), true),
            Err(TxError::IdentifierTooWide { id: 0x800 })
        );
        assert!(encode(CAN_SFF_MASK, &datagram(1), true).is_ok());
    }

    #[test]
    fn round_trip_every_length() {
        for len in 0..=FD_MTU as usize {
            let sent = datagram(len);
            let (frame, wire) = encode(0x101, &sent, true).unwrap();
            let mut out = [0u8; FD_MAX_DLEN];
            let (n, sender) = decode(&frame, wire, 0x100, 0, 0, &mut out).unwrap();
            assert_eq!(n, len);
            assert_eq!(sender, 0x101);
            assert_eq!(&out[..n], &sent[..]);
        }
    }

    #[test]
    fn round_trip_classic() {
        for len in 0..=CLASSIC_MTU as usize {
            let sent = datagram(len);
            let (frame, wire) = encode(0x101, &sent, false).unwrap();
            let mut out = [0u8; FD_MAX_DLEN];
            let (n, _sender) = decode(&frame, wire, 0x100, 0, 0, &mut out).unwrap();
            assert_eq!((n, &out[..n]), (len, &sent[..]));
        }
    }

    #[test]
    fn padding_between_the_datagram_and_the_frame_end_is_zero() {
        // A 12-byte datagram occupies 13 bytes and travels in a 16-byte frame.
        let (frame, _) = encode(0x101, &datagram(12), true).unwrap();
        assert_eq!(frame.len, 16);
        assert!(frame.data[13..].iter().all(|b| *b == 0));
    }

    #[test]
    fn own_frames_are_dropped() {
        let (frame, wire) = encode(0x100, &datagram(4), true).unwrap();
        let mut out = [0u8; FD_MAX_DLEN];
        assert_eq!(
            decode(&frame, wire, 0x100, 0, 0, &mut out),
            Err(RxDrop::OwnFrame)
        );
    }

    #[test]
    fn a_zero_mask_accepts_every_identifier() {
        let (frame, wire) = encode(0x7FF, &datagram(4), true).unwrap();
        let mut out = [0u8; FD_MAX_DLEN];
        assert!(decode(&frame, wire, 0x100, 0, 0, &mut out).is_ok());
    }

    #[test]
    fn a_nonzero_mask_rejects_outside_the_band() {
        let mut out = [0u8; FD_MAX_DLEN];
        // Band 0x100..=0x1FF.
        let (inside, wire) = encode(0x1AB, &datagram(4), true).unwrap();
        assert!(decode(&inside, wire, 0x100, 0x100, 0x700, &mut out).is_ok());

        let (outside, wire) = encode(0x2AB, &datagram(4), true).unwrap();
        assert_eq!(
            decode(&outside, wire, 0x100, 0x100, 0x700, &mut out),
            Err(RxDrop::Filtered { sender: 0x2AB })
        );
    }

    #[test]
    fn a_read_that_is_not_a_frame_is_dropped() {
        let (frame, _) = encode(0x101, &datagram(4), true).unwrap();
        let mut out = [0u8; FD_MAX_DLEN];
        for nread in [0usize, 1, 15, 17, 71, 73] {
            assert_eq!(
                decode(&frame, nread, 0x100, 0, 0, &mut out),
                Err(RxDrop::NotAFrame { nread })
            );
        }
    }

    #[test]
    fn a_frame_with_no_length_byte_is_dropped() {
        let mut frame = Frame::zeroed();
        frame.can_id = 0x101;
        frame.len = 0;
        let mut out = [0u8; FD_MAX_DLEN];
        assert_eq!(
            decode(&frame, CANFD_MTU_WIRE, 0x100, 0, 0, &mut out),
            Err(RxDrop::NoLengthByte)
        );
    }

    #[test]
    fn a_length_byte_that_overruns_the_frame_is_dropped() {
        let (mut frame, wire) = encode(0x101, &datagram(4), true).unwrap();
        frame.data[0] = 63; // frame.len is 5, so only 4 bytes are present
        let mut out = [0u8; FD_MAX_DLEN];
        assert_eq!(
            decode(&frame, wire, 0x100, 0, 0, &mut out),
            Err(RxDrop::BadLength {
                declared: 63,
                available: 4
            })
        );
    }

    #[test]
    fn a_datagram_larger_than_the_buffer_is_dropped_not_truncated() {
        let (frame, wire) = encode(0x101, &datagram(20), true).unwrap();
        let mut out = [0u8; 8];
        assert_eq!(
            decode(&frame, wire, 0x100, 0, 0, &mut out),
            Err(RxDrop::BufferTooSmall {
                needed: 20,
                have: 8
            })
        );
    }

    #[test]
    fn wire_bytes_start_with_the_identifier_little_endian() {
        let (frame, wire) = encode(0x123, &datagram(3), true).unwrap();
        let bytes = frame.as_wire_bytes(wire);
        assert_eq!(bytes.len(), CANFD_MTU_WIRE);
        assert_eq!(&bytes[..4], &[0x23, 0x01, 0x00, 0x00]);
        assert_eq!(bytes[4], 4, "frame.len is the datagram plus its prefix");
        assert_eq!(bytes[5], CANFD_BRS);
        assert_eq!(&bytes[8..12], &[3, 1, 2, 3]);
    }
}

/// Golden frames: the exact bytes the zenoh-pico sender puts on the wire.
///
/// These are hand-derived from `_z_send_can` in
/// `src/system/unix/network.c` of the vendored zenoh-pico tree, not produced by
/// [`encode`]. That is the point — they are what makes the two implementations
/// one wire format rather than two, and an interop regression fails here rather
/// than on a bus.
///
/// Layout, for reading the tables below:
/// `[0..4]` `can_id`, little-endian and native — the C code assigns
/// `frame.can_id = sock->_id` with no byte swap; `[4]` `frame.len`;
/// `[5]` `frame.flags`; `[6]` `__res0`; `[7]` `__res1`; `[8]` the datagram
/// length prefix; `[9..]` the datagram, then zeros.
///
/// Every byte past the datagram is zero because `_z_send_can` opens with
/// `memset(&frame, 0, sizeof(frame))` and never writes the reserved fields or
/// the DLC padding afterwards.
#[cfg(test)]
mod golden {
    use super::*;

    /// Pad a frame prefix out to its full wire length with zeros.
    fn padded(prefix: &[u8], wire: usize) -> Vec<u8> {
        let mut v = prefix.to_vec();
        v.resize(wire, 0);
        v
    }

    fn assert_golden(id: u32, datagram: &[u8], fd_mode: bool, expected: &[u8]) {
        let (frame, wire) = encode(id, datagram, fd_mode).unwrap();
        assert_eq!(wire, expected.len(), "wire length");
        assert_eq!(
            frame.as_wire_bytes(wire),
            expected,
            "frame bytes for a {}-byte datagram on {:#x}",
            datagram.len(),
            id
        );
    }

    /// `_z_send_can(sock{id=0x100, fd_mode=true}, ptr, 0)`.
    ///
    /// `payload = 1`, which is `<= 8`, so `frame_len` stays 1 and only the
    /// `else if (sock->_fd_mode)` arm runs — BRS is still set.
    #[test]
    fn empty_datagram_fd() {
        assert_golden(
            0x100,
            &[],
            true,
            &padded(&[0x00, 0x01, 0x00, 0x00, 0x01, CANFD_BRS, 0x00, 0x00, 0x00], CANFD_MTU_WIRE),
        );
    }

    /// `_z_send_can(sock{id=0x100, fd_mode=true}, "\x01..\x05", 5)`.
    ///
    /// `payload = 6`, still `<= 8`, so no DLC round-up.
    #[test]
    fn short_datagram_fd() {
        assert_golden(
            0x100,
            &[1, 2, 3, 4, 5],
            true,
            &padded(
                &[
                    0x00, 0x01, 0x00, 0x00, 0x06, CANFD_BRS, 0x00, 0x00, //
                    0x05, 1, 2, 3, 4, 5,
                ],
                CANFD_MTU_WIRE,
            ),
        );
    }

    /// `_z_send_can(sock{id=0x101, fd_mode=true}, "\x01..\x0B", 11)`.
    ///
    /// `payload = 12`, and the round-up loop compares `payload <= steps[i]`, so
    /// 12 selects the 12-byte step exactly rather than rounding on to 16.
    #[test]
    fn datagram_landing_exactly_on_a_dlc_step() {
        assert_golden(
            0x101,
            &[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11],
            true,
            &padded(
                &[
                    0x01, 0x01, 0x00, 0x00, 0x0C, CANFD_BRS, 0x00, 0x00, //
                    0x0B, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11,
                ],
                CANFD_MTU_WIRE,
            ),
        );
    }

    /// `_z_send_can(sock{id=0x101, fd_mode=true}, "\x01..\x0C", 12)`.
    ///
    /// `payload = 13` rounds up to the 16-byte step, so `frame.len` is 16 while
    /// only 13 bytes are meaningful. The three pad bytes are zero, and the
    /// receiver recovers the true length from the prefix rather than the DLC —
    /// which is the whole reason the prefix exists.
    #[test]
    fn datagram_that_must_round_up() {
        assert_golden(
            0x101,
            &[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12],
            true,
            &padded(
                &[
                    0x01, 0x01, 0x00, 0x00, 0x10, CANFD_BRS, 0x00, 0x00, //
                    0x0C, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12,
                ],
                CANFD_MTU_WIRE,
            ),
        );
    }

    /// `_z_send_can(sock{id=0x200, fd_mode=true}, "\x01..\x3F", 63)`.
    ///
    /// A full-MTU datagram: `payload = 64` fills the frame exactly, so there is
    /// no padding at all.
    #[test]
    fn full_mtu_datagram_fd() {
        let datagram: Vec<u8> = (1..=63).collect();
        let mut expected = vec![0x00, 0x02, 0x00, 0x00, 0x40, CANFD_BRS, 0x00, 0x00, 0x3F];
        expected.extend_from_slice(&datagram);
        assert_eq!(expected.len(), CANFD_MTU_WIRE, "the frame is exactly full");
        assert_golden(0x200, &datagram, true, &expected);
    }

    /// `_z_send_can(sock{id=0x100, fd_mode=false}, "\x01..\x07", 7)`.
    ///
    /// Classic CAN: 16 bytes go to `write(2)`, not 72, and `flags` is zero —
    /// which is what makes the first 16 bytes of a `canfd_frame` overlay a
    /// `can_frame` correctly, since `flags` lands on `__pad`.
    #[test]
    fn classic_mode_datagram() {
        assert_golden(
            0x100,
            &[1, 2, 3, 4, 5, 6, 7],
            false,
            &[
                0x00, 0x01, 0x00, 0x00, 0x08, 0x00, 0x00, 0x00, //
                0x07, 1, 2, 3, 4, 5, 6, 7,
            ],
        );
    }

    /// The receiver's rules, in the order `_z_read_can` applies them.
    ///
    /// A datagram shorter than the DLC is legal — that is exactly what the
    /// round-up case produces — so the check is `declared > available`, not
    /// `declared != available`.
    #[test]
    fn a_datagram_shorter_than_its_dlc_is_legal() {
        let (frame, wire) = encode(0x101, &[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12], true).unwrap();
        assert_eq!(frame.len, 16);
        let mut out = [0u8; FD_MAX_DLEN];
        let (n, sender) = decode(&frame, wire, 0x100, 0, 0, &mut out).unwrap();
        assert_eq!((n, sender), (12, 0x101));
        assert_eq!(&out[..n], &[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12]);
    }

    /// `mask == 0` short-circuits, so `match` is never consulted and every
    /// identifier on the bus is admitted.
    #[test]
    fn a_zero_mask_ignores_match_entirely() {
        let (frame, wire) = encode(0x7FF, &[1], true).unwrap();
        let mut out = [0u8; FD_MAX_DLEN];
        assert!(decode(&frame, wire, 0x100, 0x555, 0, &mut out).is_ok());
    }
}
