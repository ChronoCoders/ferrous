//! Golden vectors for the confidential-amount code, generated once at e6ce2aa with
//! curve25519-dalek-ng 4.1.1 and bulletproofs 4.0.0. They pin every consensus byte this code
//! produces (commitments, range proofs, the ephemeral key and the encrypted amount) and every
//! wallet-derived scalar, so a dependency change that alters any of them fails here. Expected
//! values are never regenerated to make a failing test pass.

use super::commitments::{
    commit, decrypt_amount, encrypt_amount_with_rng, generate_range_proof_with_rng,
    verify_range_proof,
};
use crate::consensus::transaction::{BlindingFactor, RangeProof};
use crate::consensus::utxo::OutPoint;
use crate::wallet::keys::{derive_blinding, derive_view_key};
use curve25519_dalek_ng::constants::RISTRETTO_BASEPOINT_POINT;
use curve25519_dalek_ng::scalar::Scalar;
use rand_chacha::rand_core::SeedableRng;
use rand_chacha::ChaCha20Rng;

const X_ZERO: [u8; 32] = [
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
];
const X_ONE: [u8; 32] = [
    0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
];
const X_FF: [u8; 32] = [
    0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
    0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
];
const X_L_MINUS_1: [u8; 32] = [
    0xec, 0xd3, 0xf5, 0x5c, 0x1a, 0x63, 0x12, 0x58, 0xd6, 0x9c, 0xf7, 0xa2, 0xde, 0xf9, 0xde, 0x14,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x10,
];
const X_L: [u8; 32] = [
    0xed, 0xd3, 0xf5, 0x5c, 0x1a, 0x63, 0x12, 0x58, 0xd6, 0x9c, 0xf7, 0xa2, 0xde, 0xf9, 0xde, 0x14,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x10,
];
const X_L_PLUS_1: [u8; 32] = [
    0xee, 0xd3, 0xf5, 0x5c, 0x1a, 0x63, 0x12, 0x58, 0xd6, 0x9c, 0xf7, 0xa2, 0xde, 0xf9, 0xde, 0x14,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x10,
];
const X_FIXED: [u8; 32] = [
    0x2f, 0x0c, 0x7d, 0x4a, 0x9e, 0x23, 0x6c, 0x5f, 0x2e, 0x02, 0xde, 0x77, 0xe0, 0x18, 0xfe, 0xf1,
    0x34, 0x7e, 0x36, 0x5f, 0xf9, 0xd6, 0x29, 0x51, 0xa6, 0xf2, 0xd7, 0xee, 0x4d, 0xcd, 0xeb, 0x09,
];
// (2^256 - 1) mod l, little-endian: what X_FF reduces to.
const X_FF_REDUCED: [u8; 32] = [
    0x1c, 0x95, 0x98, 0x8d, 0x74, 0x31, 0xec, 0xd6, 0x70, 0xcf, 0x7d, 0x73, 0xf4, 0x5b, 0xef, 0xc6,
    0xfe, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x0f,
];
const ENC_VIEW_SCALAR: [u8; 32] = [
    0x04, 0x6c, 0xdb, 0x30, 0x11, 0xf3, 0x8d, 0x58, 0xf0, 0x63, 0xc9, 0x87, 0xf1, 0xce, 0x92, 0xcb,
    0x2a, 0xeb, 0xf0, 0xf7, 0x63, 0x6e, 0xf5, 0x8e, 0x59, 0x51, 0x5e, 0xc6, 0xae, 0xe1, 0xfd, 0x0f,
];
const ENC_SEED: [u8; 32] = [
    0xbe, 0xba, 0xce, 0x5d, 0x49, 0x48, 0x5e, 0x34, 0xa5, 0xd7, 0x5f, 0x75, 0xba, 0xbd, 0xd8, 0x69,
    0x15, 0x3d, 0x5b, 0x6c, 0xe1, 0xdf, 0xe5, 0x6b, 0x34, 0x7f, 0x0d, 0x67, 0x79, 0x4b, 0xf7, 0x39,
];
const ENC_VALUE: u64 = 123_456_789;
const WALLET_SEED: [u8; 64] = [
    0x3c, 0x94, 0x39, 0x82, 0x8c, 0xcd, 0x3e, 0xf7, 0xa1, 0xbf, 0x3f, 0x39, 0x7e, 0x95, 0xc0, 0xdc,
    0xfc, 0xd9, 0x82, 0x63, 0x8d, 0xfe, 0x72, 0x06, 0x9c, 0xcb, 0x51, 0xeb, 0x8c, 0x77, 0x9e, 0xb2,
    0xcb, 0xc7, 0xda, 0x7e, 0xa1, 0xe9, 0xb4, 0x78, 0xdf, 0x4a, 0x18, 0xbd, 0xa2, 0x7b, 0xaa, 0xd5,
    0x51, 0xf1, 0x35, 0x0a, 0xec, 0xe7, 0x20, 0x3f, 0xdd, 0x82, 0x93, 0x74, 0x66, 0x09, 0xae, 0x65,
];
const TXID_0: [u8; 32] = [
    0x99, 0x3d, 0x59, 0x08, 0x4b, 0x00, 0xda, 0xc3, 0xd4, 0xb4, 0x6f, 0x4b, 0x6d, 0x02, 0x57, 0x4f,
    0x7b, 0x2e, 0xd2, 0xe3, 0xe6, 0x25, 0xcf, 0xa3, 0x27, 0x5e, 0xbd, 0xc6, 0x7d, 0x15, 0x88, 0x06,
];
const TXID_1: [u8; 32] = [
    0xff, 0x49, 0x58, 0xbd, 0x54, 0x99, 0xfb, 0x08, 0xba, 0xca, 0x71, 0xfe, 0x85, 0xe9, 0x27, 0xe8,
    0x03, 0x9f, 0x1e, 0x23, 0xe7, 0x25, 0xcb, 0xe5, 0x1c, 0x6a, 0x37, 0x07, 0x15, 0x7b, 0xec, 0x30,
];
const TXID_2: [u8; 32] = [
    0x23, 0x2c, 0x85, 0x9e, 0x7d, 0x01, 0xcb, 0x82, 0x25, 0x73, 0xfd, 0x8a, 0xb1, 0x42, 0x64, 0xe6,
    0x70, 0x7b, 0xee, 0x37, 0xfa, 0xf7, 0xe9, 0x07, 0x41, 0xb1, 0x08, 0xbf, 0x2d, 0x43, 0x80, 0x5b,
];

macro_rules! commitment_vectors {
    ($($name:ident: $v:expr, $x:expr, $hex:expr;)*) => {
        $(
            #[test]
            fn $name() {
                assert_eq!(hex::encode(commit($v, &BlindingFactor($x)).0.as_bytes()), $hex);
            }
        )*
    };
}

commitment_vectors! {
    commit_v0_x_zero: 0, X_ZERO, "0000000000000000000000000000000000000000000000000000000000000000";
    commit_v0_x_one: 0, X_ONE, "26f6bfdaa7723b3ca6def534739755e645338f4a2db5c84615af3a93e37fd818";
    commit_v0_x_ff: 0, X_FF, "f496a0f4a2dc83eb55723af0d6870cef832e27a059a3bdebb6afeef59b15be7f";
    commit_v0_x_l_minus_1: 0, X_L_MINUS_1, "3ca5f91f033fa9d87f8bf53efd44d8d7f2ed8ec8ca14edfe01d7a32af8b0a205";
    commit_v0_x_l: 0, X_L, "0000000000000000000000000000000000000000000000000000000000000000";
    commit_v0_x_l_plus_1: 0, X_L_PLUS_1, "26f6bfdaa7723b3ca6def534739755e645338f4a2db5c84615af3a93e37fd818";
    commit_v0_x_fixed: 0, X_FIXED, "b0d4a84ef70888cfaf15b761b9b25c7d351fc5eca19d2707f70113c212e4661a";
    commit_v1_x_zero: 1, X_ZERO, "e2f2ae0a6abc4e71a884a961c500515f58e30b6aa582dd8db6a65945e08d2d76";
    commit_v1_x_one: 1, X_ONE, "0a18a1cd8926a44d5f5a5f48d7cb10163ade94a7b82989d333c60ba14c8d3e11";
    commit_v1_x_ff: 1, X_FF, "2025907e753b5a379911614b8a6d72b5be38baca390b191d2f2cfee14302e35e";
    commit_v1_x_l_minus_1: 1, X_L_MINUS_1, "ea61003e6bf04b20b08f582d48b25337431ecdf2892118ae47cdc7d8ea15ae70";
    commit_v1_x_l: 1, X_L, "e2f2ae0a6abc4e71a884a961c500515f58e30b6aa582dd8db6a65945e08d2d76";
    commit_v1_x_l_plus_1: 1, X_L_PLUS_1, "0a18a1cd8926a44d5f5a5f48d7cb10163ade94a7b82989d333c60ba14c8d3e11";
    commit_v1_x_fixed: 1, X_FIXED, "fac99ba758f460365f54ce6b60b7eb9bcda693867ac67f5e8be85db576be3c55";
    commit_v2pow32_x_zero: 1 << 32, X_ZERO, "f6ea00e8940a03ccd299838a11ddfde48488d58016dbb0d1bc8b43ca04b9fb30";
    commit_v2pow32_x_one: 1 << 32, X_ONE, "7898af27d6d546594887a2f9c1a2dccf3ceaef2a02b4e8014400701aa93d745f";
    commit_v2pow32_x_ff: 1 << 32, X_FF, "26b102309d813a5bcc3e63129885bff1f4a42cda494f4498e8e187cc8b6bf81c";
    commit_v2pow32_x_l_minus_1: 1 << 32, X_L_MINUS_1, "be5e5955cdcf6c9a2de56e5dc77ad64013cc76e7241f6ca241bb59f1d542e361";
    commit_v2pow32_x_l: 1 << 32, X_L, "f6ea00e8940a03ccd299838a11ddfde48488d58016dbb0d1bc8b43ca04b9fb30";
    commit_v2pow32_x_l_plus_1: 1 << 32, X_L_PLUS_1, "7898af27d6d546594887a2f9c1a2dccf3ceaef2a02b4e8014400701aa93d745f";
    commit_v2pow32_x_fixed: 1 << 32, X_FIXED, "c8daa7b21279d966e0d14e3bb61a2d508da0e9b8c3472d00e19fe9f051c5935a";
    commit_vmax_x_zero: u64::MAX, X_ZERO, "e83906dee86ee8b8f0435e806d3c76590411b0302236ced9cc88fface454227c";
    commit_vmax_x_one: u64::MAX, X_ONE, "68e0a93d20c1d9252698550d5c792a8e809c9299fa3ba7c8d0fb8a8348c74969";
    commit_vmax_x_ff: u64::MAX, X_FF, "de0c5a7f3d90c53e3d0679554794ac80be685b561c48eaea58223749f930be31";
    commit_vmax_x_l_minus_1: u64::MAX, X_L_MINUS_1, "88d04fc9f80ef7afc7cf28bbd9a4b0693c803c1af7c2679bacfc4be447058177";
    commit_vmax_x_l: u64::MAX, X_L, "e83906dee86ee8b8f0435e806d3c76590411b0302236ced9cc88fface454227c";
    commit_vmax_x_l_plus_1: u64::MAX, X_L_PLUS_1, "68e0a93d20c1d9252698550d5c792a8e809c9299fa3ba7c8d0fb8a8348c74969";
    commit_vmax_x_fixed: u64::MAX, X_FIXED, "b288f48fbb34665722dd90c97ad2ad23b61591fbfcd9f515df74679a1d95d719";
}

#[test]
fn commit_reduces_l_to_zero() {
    for v in [0, 1, 1 << 32, u64::MAX] {
        assert_eq!(
            commit(v, &BlindingFactor(X_L)),
            commit(v, &BlindingFactor(X_ZERO))
        );
    }
}

#[test]
fn commit_reduces_l_plus_1_to_one() {
    for v in [0, 1, 1 << 32, u64::MAX] {
        assert_eq!(
            commit(v, &BlindingFactor(X_L_PLUS_1)),
            commit(v, &BlindingFactor(X_ONE))
        );
    }
}

#[test]
fn commit_reduces_all_ff_mod_l() {
    for v in [0, 1, 1 << 32, u64::MAX] {
        assert_eq!(
            commit(v, &BlindingFactor(X_FF)),
            commit(v, &BlindingFactor(X_FF_REDUCED))
        );
    }
}

// Each proof uses blinding X_FIXED. The golden proof is checked byte for byte against a fresh
// proof from the production prover, and verified as stored, so a proof made by the old
// dependencies must verify under the new ones.
macro_rules! proof_vectors {
    ($($name:ident: $v:expr, $seed:expr, $commitment:expr, $proof:expr;)*) => {
        $(
            #[test]
            fn $name() {
                let mut rng = ChaCha20Rng::from_seed($seed);
                let made = generate_range_proof_with_rng($v, &BlindingFactor(X_FIXED), &mut rng)
                    .unwrap();
                let golden = RangeProof(hex::decode($proof).unwrap());
                assert_eq!(made, golden);
                let commitment = commit($v, &BlindingFactor(X_FIXED));
                assert_eq!(hex::encode(commitment.0.as_bytes()), $commitment);
                assert!(verify_range_proof(&commitment, &golden).is_ok());
            }
        )*
    };
}

proof_vectors! {
    proof_v0: 0, [0x5e, 0x6c, 0x66, 0x37, 0x94, 0x9e, 0x6b, 0xdc, 0xb3, 0xc6, 0xf8, 0x16, 0x30, 0x2f, 0xb2, 0x40, 0x28, 0x91, 0xd8, 0x74, 0x22, 0x4c, 0x51, 0x5b, 0xa2, 0x7a, 0xc4, 0x2d, 0x76, 0x23, 0x8d, 0xdd],
        "b0d4a84ef70888cfaf15b761b9b25c7d351fc5eca19d2707f70113c212e4661a",
        concat!(
            "822257a8b503d397e0d7ad8be066b696c3a9651eb1bbd14120cb30638da92c77",
            "86d777220f26481a6dc38c290debb39cb35cc5431459da0c6a72f5da3981aa57",
            "ce5a5a1ca2fbbc2a09ef47032de446499ca2520ee5106c0adb78698af7b7a241",
            "c27154eb9c86b0e4ad6c82abc01054a699e1603d2ccebfe3ce0075f85bfe4b1f",
            "a3465a09c0c9cc0cf018ddf26a0e57ad59e6a9a0e420027fe0ba911dbb4a8601",
            "dcdd49979b34e05b552dec2673747b54e939a141958ecf2ae7dc9c2c29519c0d",
            "ff3e79d2af0844b2e746d9b743deba76197e248f95d488de8d366c3df05f8c03",
            "c41a5d6ad8b99763accc2a8d7dc416f4485383d27471cf9533204b8a9ec4864b",
            "da5b7a7355e5ca930344ab32cc31a9413c493c56d444b228f2d67c4317e10f77",
            "94b5607c528b8dd30adb5bb407ab992a0bd942c70f294d2eff89671f986e2e2c",
            "fca7979da8999ae70bc881f5d6ee86306ce7c8a8fcd94ede2b6ff232d9ae3251",
            "26e0166fc9e6ebd6b658dd5c693e3c34a926824dd81b87478b112372cc925d58",
            "38699c22831d7d260c7037ce4b0d4fb921e533b3c0ceef48c2a6172ab03afc51",
            "bac85777c22b71f53b727ba28e3ebfe4108f9bc117f45bc62bde21bd47a3125d",
            "ec0f9a37f90526de59ea213554a26e5d0229ce336762a2f05a50e5976a286221",
            "3ca79e4fb55c40b0a34722fc1fbe576b6aebefcccf93d5c375cd01f982806b70",
            "28a5a23b5b449afbfce020f41e0c7d68486ccc179969980959161820d1ebdc7f",
            "fc00ad95af5554222c4a5b6906a4fa914db121b7a00a100e4adcf955e5bcd625",
            "1660e4141fa0a386d2086ee8b0d33a596f6662eef15a395403f7e5e767a45c72",
            "f8c3812330acbbcdd5160dd1e8377d23253da37d9ebd0eaf7057a96421a7b70c",
            "d3fc5bbee03060e4e7a5bcaed6b357ec783a49201abc5f90966f42ac0b6d4103",
        );
    proof_v1: 1, [0x30, 0x55, 0xf4, 0xb2, 0x14, 0x0e, 0x25, 0xfe, 0x9c, 0xed, 0xaf, 0x3c, 0x0f, 0x63, 0x41, 0x3e, 0xd2, 0x4d, 0xdf, 0x8c, 0xf4, 0xe4, 0x9d, 0x26, 0xa7, 0x83, 0xaf, 0x9f, 0xb1, 0x73, 0xbb, 0x64],
        "fac99ba758f460365f54ce6b60b7eb9bcda693867ac67f5e8be85db576be3c55",
        concat!(
            "a673da7613126666be77aa39534036f24cf9f212bf8dc4ca9143af312ac0fe7f",
            "cc1ce8ff4f26433f62bec2579a6de52938a94755da2fcae51ede488304c1f22b",
            "74fde2d15a723c59723ac812ee0d4232477e44766dc5227058816e811f981167",
            "76771180acc54a74bb7f9633498c22553cf95c6f7d041f7cfdf6e3c61d52c51b",
            "ae38d8c1776a4c1accbcaf9a22fc90ebcff351709dc6d77d5d2ecf2bf7f9a107",
            "f10c5674c5503fb42df9d78f326f1ed8873b4e6c109036a5e339d20ac0e98e03",
            "0a824fc0a809deb68ea50455558a09da72361584f9a18b16c0644327ac8aab05",
            "de76eca45646073a0dd8e42e8d4970ded5aac931338a7e86f5712a37e40da226",
            "82acefbdea529f24fdf124c2b4305bd24a9fb3a963abf13e24e789e66a75265f",
            "367ba186e3babfa13d82847bd0d05f6a9a1c2efa0210d750ab6d898026f37a7f",
            "90589bf981c100aedb7dc5ba3cc9005239757375a1d8ed1af40cf3752766c53c",
            "1e2f068d5dac3974289613f1fa2af087b4b956d974ed2adc71de488c99ed4422",
            "ee2e5db200ba7b2124b480d7ef75e582e0689e54d7818a6c562732d676829e5f",
            "8ec986a0511c0f4bae2683767e20c8322b795469a1fa0c439568b8ce92d7b846",
            "ac907be762ed71e49e99889dc00be50647dd5edaa9b543114aa56a8d9b831560",
            "8836c386ceed2ce1935136b72eec3a7849e0d141d0cf73378c3a273ff6794f08",
            "668f9953cc2d9be9e96d03971765aff91efddeb0b6e21bccb366a28c18e07c6f",
            "705b0ea5fce5a20e5b876c25147aa47ab7156247458b299edbc08ffe9efa1f70",
            "0016671f2d794c05fcf030e3548df6a4fbf975d2ef186bb5c121577fed02652d",
            "732acb0290b767e50cb1546ccf2798baa0677005f098dd6e634571cecbd65205",
            "b2cfc0e65996a67f49f433a70a45598be1ddc7fa5640cceced2b23992b1e090c",
        );
    proof_v2pow63: 1 << 63, [0xc2, 0xad, 0x7d, 0xca, 0xe8, 0x06, 0x73, 0x80, 0x2f, 0x1a, 0x3f, 0x00, 0xd6, 0xdd, 0x0b, 0x32, 0xf5, 0x12, 0x1d, 0x0c, 0x67, 0x21, 0x11, 0x6e, 0x1c, 0xb3, 0x31, 0x01, 0x4c, 0xcc, 0xc9, 0xad],
        "6201963c48ed14c79cbb1c487c9293bc170aa54c93501d434fc686976e38ed21",
        concat!(
            "2607abd3ec147f33e29a9c9dbbcc791fa7534df10a1bddbff5faa2372ad7275d",
            "1e00ca3eec5974357489ccac40783a06b77b33f09d391c3e08f91fc7458c8555",
            "ea92e04844a7dd07b9eb0bf36e71f5cc638b0f06d1385df117c26eafd112b65b",
            "5e095c93ed507c54ccebe514fd33b9708883b6406d2c076522997e1bf2f90f0e",
            "d2c484def50343e6e9d6d4e668913f7044c73017af0b3850b6a9f10022320109",
            "bd90d85a6eeb5cb9b4e9d89fe3251a06ab38c43eb1575edc7a98d6204726b50a",
            "63736ed1590b783db154411a6222c07371d59040ef956cce630b3969b16be00b",
            "b2d1e548998df35b3b74b54e536070a2d09f0b7cec93b2c975fa81d152d28f3c",
            "e0082aabd305caa73e9d59cfd2e45b3453f068fbdfd5ffdd87e17798c9cab826",
            "0a492aee4d2987463afc8a06204ee2177dabbb234d544610f24fdd83ab4cbd55",
            "5e8d90d03d803ec8c77360e5414cba99c47691518b0d1872e450008a9723df77",
            "d68409587b0470a624975588c118d5fc90ed8082b63c18971e59d09ef1bd4436",
            "c0ce01b09ee4b82ac4d7af91109100b4e4a2f247790b9846398c23dde5ee4b06",
            "50349e615f9bac12f8abcd0e20d6ef4e58815ea8508921dc15d0c7a4c8bc1144",
            "bc53f31ec1d8ba61aa4cb0e9a722972b676e199c0c15a384c823127cd36abf55",
            "827c819a8dab589fb755af38a744997c2f983f6d8207d8e8df1c7f0de4ce417d",
            "40cecad9f69d9631081514d9f32be202293eb58a4f5dc0e5ab0092d3e9210579",
            "08e392267b570834bbe971230aa6d78dbb465a788eac6d7b1b9855093760dd4b",
            "84575ee7f3e704b136628524de5b036d624cbfa7418d2fd3e85638a5db2a1e7d",
            "9f05f56421552a65b652ba1f84c0e05d515bc83f29ab5f83faa0dccd9389a10f",
            "d0f49612776145bc3551736e64ddb434e2a02c3501b047ca0447bfd79455b709",
        );
    proof_vmax: u64::MAX, [0x7b, 0x6a, 0x5c, 0xab, 0x9a, 0x6d, 0xa7, 0x36, 0xc3, 0xd2, 0xde, 0x9d, 0x40, 0xe0, 0x44, 0xfd, 0x80, 0x34, 0x9b, 0x4c, 0xf4, 0x70, 0x29, 0xc9, 0xd9, 0xde, 0x55, 0x09, 0x08, 0x89, 0xaf, 0xaa],
        "b288f48fbb34665722dd90c97ad2ad23b61591fbfcd9f515df74679a1d95d719",
        concat!(
            "18545fb8b6155293ee52daa6a42cd20fb651e00b27b2ae3859778e9835199348",
            "341cdee4bf12e7efb2b99f58e36f72af1ed6e393df0911500fb01f57882df80c",
            "f804d5f174c759eba4d4573510c7db8a7b8a3d569c1f391e6744d7d107837a10",
            "120c5ca72a2ebc7f0bab72dea83ae6aac5f0f7e46f692751988dee7a13596111",
            "4d1dd2988eb04d4ec2fdec9ec6cb96cc987c8177f1209e0996c56554d6eea701",
            "7a702dfce688643b94357d72f813ee0faa9e571e6bef94cade944b5f13c37600",
            "162ed873ab43dde3431d2cdaeaffe948c0207e35f539a9cc903d8455332d010d",
            "140eeb5df7fa2da22e7055b45230cd867d6b3252f9bb72a4446eb26b42061514",
            "bc0e7000d3829fc833a87f44fd431b9aa0ca3470d49441e83a12ffb8c2ebbb7c",
            "8e3762dfbc7c5d1db1fd9d40ad8f78070b228f1fa44cefe3f0d0c7c88f5e1272",
            "362735bb51b56c333586243694c75123e725ec72109a943c0da336a78769f53d",
            "4c67638c94e0003ba35d8e635fc6667934c041e490dcba589e34b5146a1a376c",
            "da444c51a51649804b7e846772364a69951a94dd6fb54924e06d56e2e5c13c1d",
            "e88dcc336434970e8b38ca39c018a8d50b11667e09cc94269cd482888f7e913e",
            "982d702c07d81e858b7e15d2afb5a90820992d7a6eff5284d8f468f2ee064302",
            "a80481cae2f65d142607d5df50c563d96037577d3b11e37d520c52e70fa90604",
            "1c81b52281858226d2c40409530476600840b3890acfa320e77d3e3560717f1a",
            "f848bc14c71988644b586cb496b7288021bf43995055983112d65bf01428294d",
            "8e7bc3cc97f99087e1094bcecda317a49f77ff0d62dc2d2c9f174f3544dac54a",
            "99310b906a02e1b00f01c6ea88b6f720a186dad82c607af9cb067cb330f1cb06",
            "fa6223dd0282c9282640955f8ec1d521db02dfbb2b03873fe120d170c1c71402",
        );
}

#[test]
fn amount_encryption() {
    let view_scalar = Scalar::from_bytes_mod_order(ENC_VIEW_SCALAR);
    let view_pubkey = view_scalar * RISTRETTO_BASEPOINT_POINT;
    let mut rng = ChaCha20Rng::from_seed(ENC_SEED);
    let blinding = BlindingFactor(X_FIXED);
    let (ciphertext, ephemeral) =
        encrypt_amount_with_rng(ENC_VALUE, &blinding, &view_pubkey, &mut rng);
    assert_eq!(
        hex::encode(ephemeral),
        "9661fbde715fbc4b1c92678fe048c7e4c0a53c1b6dd1a5c41b5d0a5d09a0620c"
    );
    assert_eq!(
        hex::encode(&ciphertext),
        concat!(
            "d28f5ab8fe0cfea2be30bf6facb15de3b0e9498919e2c0b248507ad463514942",
            "9ff67e65dd15c559",
        )
    );
    assert_eq!(
        decrypt_amount(&ciphertext, &ephemeral, &view_scalar),
        Some((ENC_VALUE, blinding))
    );
}

macro_rules! blinding_vectors {
    ($($name:ident: $txid:expr, $vout:expr, $hex:expr;)*) => {
        $(
            #[test]
            fn $name() {
                let outpoint = OutPoint { txid: $txid, vout: $vout };
                assert_eq!(hex::encode(derive_blinding(&WALLET_SEED, &outpoint).0), $hex);
            }
        )*
    };
}

blinding_vectors! {
    derive_blinding_0: TXID_0, 0, "f76d45a9e0e9ca5dcda3b51350a288d72c2a2a6875c9f29b62cfae28312e780f";
    derive_blinding_1: TXID_1, 1, "9004b8a79595fc98dc5187ce5586e97287fbdca362abe15e222e8605b1e3570e";
    derive_blinding_2: TXID_2, 2, "d1b9865787847abd617e4a483c1bcfd88d26d48ca67019cb6eb46d4b6ccca107";
}

#[test]
fn view_key_derivation() {
    let (scalar, point) = derive_view_key(&WALLET_SEED);
    assert_eq!(
        hex::encode(scalar.to_bytes()),
        "e0fd69bf6f0e9842dcbeab3434a3a4dc983568f0cee0f610ac4359018a0b870c"
    );
    assert_eq!(
        hex::encode(point.compress().as_bytes()),
        "f8851b472a5f49ec17384aad563ac7f2cdbe50c5357bdeb64f0d393e1b845f35"
    );
}
