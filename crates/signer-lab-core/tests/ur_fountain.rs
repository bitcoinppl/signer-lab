use bitcoin::{
    Amount, OutPoint, Psbt, ScriptBuf, Sequence, Transaction, TxIn, TxOut, Txid, Witness,
    absolute::LockTime, hashes::Hash as _, psbt::raw, transaction::Version,
};
use cove_ur::{CryptoPsbt, Ur};
use foundation_ur::Decoder;
use signer_lab_core::{
    QrDensity, QrFramePosition, QrFrames, QrScanProgress, QrScanner, QrTransport,
};

/// Characters QR alphanumeric mode can encode
const QR_ALPHANUMERIC: &str = "0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ $%*+-./:";

/// The fragment a phone in the field could not read from the screen
const UNREADABLE_FRAGMENT: usize = 15;

/// A PSBT about the size of the signed 47-input testnet PSBT that stalled Cove's scanner
fn large_psbt() -> Psbt {
    psbt_with_inputs(47, 450)
}

fn psbt_with_inputs(input_count: u8, bytes_per_input: usize) -> Psbt {
    let input = (0..input_count)
        .map(|index| TxIn {
            previous_output: OutPoint::new(Txid::from_byte_array([index; 32]), u32::from(index)),
            script_sig: ScriptBuf::new(),
            sequence: Sequence::ENABLE_RBF_NO_LOCKTIME,
            witness: Witness::new(),
        })
        .collect();

    let mut psbt = Psbt::from_unsigned_tx(Transaction {
        version: Version::TWO,
        lock_time: LockTime::ZERO,
        input,
        output: vec![TxOut {
            value: Amount::from_sat(10_000),
            script_pubkey: ScriptBuf::new(),
        }],
    })
    .expect("unsigned transaction is a valid PSBT");

    // signatures and derivations dominate a signed PSBT's size, and unknown entries stand in for them
    for (index, input) in (0_u8..).zip(&mut psbt.inputs) {
        input.unknown.insert(
            raw::Key {
                type_value: 0xf0,
                key: vec![index],
            },
            vec![index; bytes_per_input],
        );
    }

    psbt
}

fn fragment_count(frames: &QrFrames) -> usize {
    let Some(QrFramePosition::Fragment { count, .. }) = frames.position(0) else {
        panic!("a UR cycle starts with its fragment pass");
    };

    count
}

/// Frames a receiver reads while one fragment's frame is unreadable on every loop
fn without_fragment(frames: &QrFrames, fragment: usize) -> impl Iterator<Item = &str> {
    frames
        .frames()
        .iter()
        .enumerate()
        .filter(move |(index, _)| *index != fragment - 1)
        .map(|(_, frame)| frame.as_str())
}

fn without_unreadable_fragment(frames: &QrFrames) -> impl Iterator<Item = &str> {
    without_fragment(frames, UNREADABLE_FRAGMENT)
}

/// Decodes the way Cove's scanner does, through `cove_ur` case normalization and the fountain decoder
fn cove_decode<'a>(frames: impl Iterator<Item = &'a str>) -> Option<Psbt> {
    let mut decoder = Decoder::default();
    for frame in frames {
        let ur = Ur::parse(frame).expect("Cove parses the uppercase frame");
        decoder
            .receive(ur.to_foundation_ur().expect("Cove normalizes the frame"))
            .expect("Cove accepts the frame");

        if decoder.is_complete() {
            let cbor = decoder
                .message()
                .expect("complete decoder has a valid message")
                .expect("complete decoder has a message");
            let psbt = CryptoPsbt::from_cbor(cbor).expect("message is a crypto-psbt");
            return Some(psbt.psbt().clone());
        }
    }

    None
}

fn signer_lab_decode<'a>(frames: impl Iterator<Item = &'a str>) -> Option<Psbt> {
    let mut scanner = QrScanner::new();
    frames
        .map(|frame| {
            scanner
                .receive(frame)
                .expect("Signer Lab accepts the frame")
        })
        .find_map(|progress| match progress {
            QrScanProgress::Complete { psbt, .. } => Some(psbt),
            QrScanProgress::InProgress { .. } => None,
        })
}

#[test]
fn ur_cycle_completes_when_one_fragment_is_never_read() {
    let expected = large_psbt();
    let frames = QrFrames::from_psbt(&expected, QrTransport::Ur, QrDensity::default())
        .expect("large PSBT encodes as UR");
    let fragments = fragment_count(&frames);
    assert!(
        fragments > UNREADABLE_FRAGMENT,
        "test PSBT needs more than {UNREADABLE_FRAGMENT} fragments, got {fragments}"
    );

    // the fragment pass alone cannot finish without the missing fragment, as in the field report
    let fragment_pass = without_unreadable_fragment(&frames).take(fragments - 1);
    assert_eq!(signer_lab_decode(fragment_pass), None);

    assert_eq!(
        signer_lab_decode(without_unreadable_fragment(&frames)),
        Some(expected.clone())
    );
    assert_eq!(
        cove_decode(without_unreadable_fragment(&frames)),
        Some(expected)
    );
}

#[test]
fn every_fragment_is_mixed_before_the_cycle_repeats() {
    // with only the minimum 16 fountain parts, none of them mixes fragment 2 of this PSBT
    let expected = psbt_with_inputs(1, 1_352);
    let frames = QrFrames::from_psbt(&expected, QrTransport::Ur, QrDensity::default())
        .expect("PSBT encodes as UR");
    let fragments = fragment_count(&frames);
    assert_eq!(fragments, 8);

    for fragment in 1..=fragments {
        assert!(
            signer_lab_decode(without_fragment(&frames, fragment)).as_ref() == Some(&expected),
            "cycle without fragment {fragment} does not decode"
        );
    }
}

#[test]
fn ur_frames_use_only_qr_alphanumeric_characters() {
    let frames = QrFrames::from_psbt(&large_psbt(), QrTransport::Ur, QrDensity::default())
        .expect("large PSBT encodes as UR");

    for frame in frames.frames() {
        assert!(
            frame.chars().all(|char| QR_ALPHANUMERIC.contains(char)),
            "frame needs QR byte mode: {frame}"
        );
    }
}

#[test]
fn single_fragment_ur_has_no_fountain_parts() {
    let psbt = Psbt::from_unsigned_tx(Transaction {
        version: Version::TWO,
        lock_time: LockTime::ZERO,
        input: Vec::new(),
        output: Vec::new(),
    })
    .expect("empty transaction is a valid PSBT");
    let frames = QrFrames::from_psbt(&psbt, QrTransport::Ur, QrDensity::default())
        .expect("empty PSBT encodes as UR");

    assert_eq!(frames.frames().len(), 1);
    assert_eq!(
        frames.position(0),
        Some(QrFramePosition::Fragment {
            number: 1,
            count: 1
        })
    );
}
