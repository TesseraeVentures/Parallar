//! Sprint 0 — Spike 2: guest↔host Poseidon parity.
//!
//! Soroban's `poseidon2_permutation` host fn is adapted from HorizenLabs/poseidon2.
//! `soroban-env-host`'s own test asserts that, for the BN254 t=3 instance,
//! `poseidon2_permutation([0,1,2])` == the EXPECTED vector below.
//!
//! The RISC Zero settlement guest will compute Poseidon2 with the `zkhash` crate
//! (HorizenLabs/poseidon2). If `zkhash` reproduces that exact vector, the guest's
//! commitments/roots are byte-identical to anything the chain recomputes via the
//! host fn — i.e. guest↔host parity holds. This is the architecture-blocking gate.

#[cfg(test)]
mod tests {
    use ark_ff::{BigInteger, PrimeField};
    use zkhash::fields::bn256::FpBN256 as Scalar;
    use zkhash::poseidon2::poseidon2::Poseidon2;
    use zkhash::poseidon2::poseidon2_instance_bn256::POSEIDON2_BN256_PARAMS;

    fn to_hex_be(x: &Scalar) -> String {
        let bytes = x.into_bigint().to_bytes_be();
        let mut s = String::from("0x");
        for _ in 0..(32 - bytes.len()) {
            s.push_str("00");
        }
        for b in bytes {
            s.push_str(&format!("{:02x}", b));
        }
        s
    }

    #[test]
    fn poseidon2_bn254_parity_with_soroban_hostfn() {
        let perm = Poseidon2::new(&POSEIDON2_BN256_PARAMS);
        let input = vec![Scalar::from(0u64), Scalar::from(1u64), Scalar::from(2u64)];
        let out = perm.permutation(&input);
        let got: Vec<String> = out.iter().map(to_hex_be).collect();
        for (i, g) in got.iter().enumerate() {
            println!("zkhash out[{i}] = {g}");
        }

        // Soroban host-fn ground truth (soroban-env-host test_poseidon2_bn254_hostfn_success)
        let expected = vec![
            "0x0bb61d24daca55eebcb1929a82650f328134334da98ea4f847f760054f4a3033".to_string(),
            "0x303b6f7c86d043bfcbcc80214f26a30277a15d3f74ca654992defe7ff8d03570".to_string(),
            "0x1ed25194542b12eef8617361c3ba7c52e660b145994427cc86296242cf766ec8".to_string(),
        ];
        assert_eq!(
            got, expected,
            "zkhash Poseidon2 must match Soroban host-fn output (guest↔host parity)"
        );
    }
}
