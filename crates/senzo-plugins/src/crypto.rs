use aes::{
    Aes128, Aes192, Aes256,
    cipher::{Block, BlockDecrypt, BlockEncrypt, KeyInit},
};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use hmac::{Hmac, Mac};
use md5::Md5;
use pbkdf2::pbkdf2_hmac;
use ring::{
    rand::{SecureRandom, SystemRandom},
    signature::{
        ECDSA_P256_SHA256_ASN1, ECDSA_P256_SHA256_ASN1_SIGNING,
        RSA_PKCS1_2048_8192_SHA256, RSA_PKCS1_SHA256, EcdsaKeyPair,
        RsaKeyPair, RsaPublicKeyComponents, UnparsedPublicKey,
    },
};
use rquickjs::{Object, prelude::Func};
use sha1::Sha1;
use sha2::{Digest, Sha256, Sha384, Sha512};

const MAX_RANDOM_BYTES: usize = 65_536;
const MAX_CRYPTO_INPUT_BYTES: usize = 4 * 1024 * 1024;
const MAX_PBKDF2_ITERATIONS: u32 = 1_000_000;
const MAX_PBKDF2_OUTPUT_BYTES: usize = 512;

type CryptoResult<T> = Result<T, String>;

fn normalize_hash(name: &str) -> CryptoResult<&'static str> {
    let normalized = name
        .to_ascii_uppercase()
        .replace(['-', '_', '/', ' '], "");
    match normalized.as_str() {
        "MD5" => Ok("MD5"),
        "SHA1" => Ok("SHA1"),
        "SHA256" => Ok("SHA256"),
        "SHA384" => Ok("SHA384"),
        "SHA512" => Ok("SHA512"),
        _ => Err(format!("Unsupported hash algorithm: {name}")),
    }
}

fn digest_bytes(algorithm: &str, data: &[u8]) -> CryptoResult<Vec<u8>> {
    if data.len() > MAX_CRYPTO_INPUT_BYTES {
        return Err("Cryptographic input exceeds the limit".to_owned());
    }
    Ok(match normalize_hash(algorithm)? {
        "MD5" => Md5::digest(data).to_vec(),
        "SHA1" => Sha1::digest(data).to_vec(),
        "SHA256" => Sha256::digest(data).to_vec(),
        "SHA384" => Sha384::digest(data).to_vec(),
        "SHA512" => Sha512::digest(data).to_vec(),
        _ => unreachable!(),
    })
}

fn hmac_bytes(algorithm: &str, key: &[u8], data: &[u8]) -> CryptoResult<Vec<u8>> {
    if key.len() > MAX_CRYPTO_INPUT_BYTES || data.len() > MAX_CRYPTO_INPUT_BYTES {
        return Err("Cryptographic input exceeds the limit".to_owned());
    }
    macro_rules! compute {
        ($digest:ty) => {{
            let mut mac = <Hmac<$digest> as Mac>::new_from_slice(key)
                .map_err(|error| error.to_string())?;
            mac.update(data);
            mac.finalize().into_bytes().to_vec()
        }};
    }
    Ok(match normalize_hash(algorithm)? {
        "MD5" => compute!(Md5),
        "SHA1" => compute!(Sha1),
        "SHA256" => compute!(Sha256),
        "SHA384" => compute!(Sha384),
        "SHA512" => compute!(Sha512),
        _ => unreachable!(),
    })
}

fn pbkdf2_bytes(
    algorithm: &str,
    password: &[u8],
    salt: &[u8],
    iterations: u32,
    output_bytes: usize,
) -> CryptoResult<Vec<u8>> {
    if iterations == 0
        || iterations > MAX_PBKDF2_ITERATIONS
        || output_bytes == 0
        || output_bytes > MAX_PBKDF2_OUTPUT_BYTES
        || password.len() > MAX_CRYPTO_INPUT_BYTES
        || salt.len() > MAX_CRYPTO_INPUT_BYTES
    {
        return Err("PBKDF2 parameters exceed the runtime limits".to_owned());
    }
    let mut output = vec![0u8; output_bytes];
    match normalize_hash(algorithm)? {
        "MD5" => pbkdf2_hmac::<Md5>(password, salt, iterations, &mut output),
        "SHA1" => pbkdf2_hmac::<Sha1>(password, salt, iterations, &mut output),
        "SHA256" => pbkdf2_hmac::<Sha256>(password, salt, iterations, &mut output),
        "SHA384" => pbkdf2_hmac::<Sha384>(password, salt, iterations, &mut output),
        "SHA512" => pbkdf2_hmac::<Sha512>(password, salt, iterations, &mut output),
        _ => unreachable!(),
    }
    Ok(output)
}

fn aes_block_mode<C>(
    key: &[u8],
    iv: &[u8],
    input: &[u8],
    encrypt: bool,
    cbc: bool,
    no_padding: bool,
) -> CryptoResult<Vec<u8>>
where
    C: KeyInit + BlockEncrypt + BlockDecrypt,
{
    if input.len() > MAX_CRYPTO_INPUT_BYTES {
        return Err("AES input exceeds the runtime limit".to_owned());
    }
    if cbc && iv.len() != 16 {
        return Err("AES-CBC requires a 16-byte IV".to_owned());
    }
    let cipher = C::new_from_slice(key).map_err(|_| "Invalid AES key length".to_owned())?;
    let mut output = input.to_vec();

    if encrypt {
        if no_padding {
            if !output.len().is_multiple_of(16) {
                return Err("AES no-padding input must be a multiple of 16 bytes".to_owned());
            }
        } else {
            let padding = 16 - (output.len() % 16);
            output.extend(std::iter::repeat_n(padding as u8, padding));
        }
        let mut previous = [0u8; 16];
        if cbc {
            previous.copy_from_slice(iv);
        }
        for chunk in output.chunks_exact_mut(16) {
            let mut block = Block::<C>::clone_from_slice(chunk);
            if cbc {
                for (byte, prior) in block.iter_mut().zip(previous.iter()) {
                    *byte ^= *prior;
                }
            }
            cipher.encrypt_block(&mut block);
            chunk.copy_from_slice(&block);
            if cbc {
                previous.copy_from_slice(chunk);
            }
        }
    } else {
        if !output.len().is_multiple_of(16) {
            return Err("AES ciphertext must be a multiple of 16 bytes".to_owned());
        }
        let mut previous = [0u8; 16];
        if cbc {
            previous.copy_from_slice(iv);
        }
        for chunk in output.chunks_exact_mut(16) {
            let mut current = [0u8; 16];
            current.copy_from_slice(chunk);
            let mut block = Block::<C>::clone_from_slice(chunk);
            cipher.decrypt_block(&mut block);
            if cbc {
                for (byte, prior) in block.iter_mut().zip(previous.iter()) {
                    *byte ^= *prior;
                }
                previous = current;
            }
            chunk.copy_from_slice(&block);
        }
        if !no_padding && !output.is_empty() {
            let padding = *output.last().unwrap_or(&0) as usize;
            if padding == 0
                || padding > 16
                || padding > output.len()
                || !output[output.len() - padding..]
                    .iter()
                    .all(|byte| *byte as usize == padding)
            {
                return Err("Invalid AES padding".to_owned());
            }
            output.truncate(output.len() - padding);
        }
    }
    Ok(output)
}

fn aes_encrypt_block<C>(cipher: &C, input: &[u8; 16]) -> [u8; 16]
where
    C: BlockEncrypt,
{
    let mut block = Block::<C>::clone_from_slice(input);
    cipher.encrypt_block(&mut block);
    let mut output = [0u8; 16];
    output.copy_from_slice(&block);
    output
}

fn ghash_multiply(left: u128, right: u128) -> u128 {
    let mut product = 0u128;
    let mut multiplicand = right;
    for bit in 0..128 {
        if left & (1u128 << (127 - bit)) != 0 {
            product ^= multiplicand;
        }
        let least_significant_bit = multiplicand & 1;
        multiplicand >>= 1;
        if least_significant_bit != 0 {
            multiplicand ^= 0xe1000000000000000000000000000000;
        }
    }
    product
}

fn ghash_blocks(mut state: u128, data: &[u8], hash_key: u128) -> u128 {
    for chunk in data.chunks(16) {
        let mut block = [0u8; 16];
        block[..chunk.len()].copy_from_slice(chunk);
        state = ghash_multiply(state ^ u128::from_be_bytes(block), hash_key);
    }
    state
}

fn ghash_lengths(state: u128, aad_bits: u64, data_bits: u64, hash_key: u128) -> u128 {
    let lengths = ((aad_bits as u128) << 64) | data_bits as u128;
    ghash_multiply(state ^ lengths, hash_key)
}

fn gcm_counter_crypt<C>(cipher: &C, initial_counter: u128, input: &[u8]) -> Vec<u8>
where
    C: BlockEncrypt,
{
    let mut output = input.to_vec();
    let mut counter = initial_counter;
    for chunk in output.chunks_mut(16) {
        let next_counter = (counter as u32).wrapping_add(1);
        counter = (counter & !0xffff_ffffu128) | u128::from(next_counter);
        let stream = aes_encrypt_block(cipher, &counter.to_be_bytes());
        for (byte, mask) in chunk.iter_mut().zip(stream.iter()) {
            *byte ^= *mask;
        }
    }
    output
}

fn gcm_authentication_tag<C>(
    cipher: &C,
    initial_counter: u128,
    ciphertext: &[u8],
    hash_key: u128,
) -> [u8; 16]
where
    C: BlockEncrypt,
{
    let state = ghash_blocks(0, ciphertext, hash_key);
    let state = ghash_lengths(state, 0, (ciphertext.len() as u64) * 8, hash_key);
    let mask = u128::from_be_bytes(aes_encrypt_block(cipher, &initial_counter.to_be_bytes()));
    (mask ^ state).to_be_bytes()
}

fn aes_gcm<C>(
    key: &[u8],
    iv: &[u8],
    input: &[u8],
    encrypt: bool,
) -> CryptoResult<Vec<u8>>
where
    C: KeyInit + BlockEncrypt,
{
    if iv.is_empty() || iv.len() > MAX_CRYPTO_INPUT_BYTES {
        return Err("AES-GCM requires a bounded, non-empty nonce".to_owned());
    }
    if input.len() > MAX_CRYPTO_INPUT_BYTES {
        return Err("AES input exceeds the runtime limit".to_owned());
    }
    let cipher = C::new_from_slice(key).map_err(|_| "Invalid AES-GCM key".to_owned())?;
    let zero_block = [0u8; 16];
    let hash_key = u128::from_be_bytes(aes_encrypt_block(&cipher, &zero_block));

    // GCM's 96-bit fast path and its general IV derivation are both supported.
    let initial_counter = if iv.len() == 12 {
        let mut block = [0u8; 16];
        block[..12].copy_from_slice(iv);
        block[15] = 1;
        u128::from_be_bytes(block)
    } else {
        let state = ghash_blocks(0, iv, hash_key);
        ghash_lengths(state, 0, (iv.len() as u64) * 8, hash_key)
    };

    if encrypt {
        let mut ciphertext = gcm_counter_crypt(&cipher, initial_counter, input);
        let tag = gcm_authentication_tag(&cipher, initial_counter, &ciphertext, hash_key);
        ciphertext.extend_from_slice(&tag);
        Ok(ciphertext)
    } else {
        if input.len() < 16 {
            return Err("AES-GCM ciphertext is shorter than its authentication tag".to_owned());
        }
        let (ciphertext, supplied_tag) = input.split_at(input.len() - 16);
        let expected_tag =
            gcm_authentication_tag(&cipher, initial_counter, ciphertext, hash_key);
        let mut difference = 0u8;
        for (expected, actual) in expected_tag.iter().zip(supplied_tag.iter()) {
            difference |= expected ^ actual;
        }
        if difference != 0 {
            return Err("AES-GCM authentication failed".to_owned());
        }
        Ok(gcm_counter_crypt(&cipher, initial_counter, ciphertext))
    }
}

fn aes_bytes(
    mode: &str,
    key: &[u8],
    iv: &[u8],
    input: &[u8],
    encrypt: bool,
) -> CryptoResult<Vec<u8>> {
    let normalized = mode.to_ascii_uppercase().replace(['_', ' '], "");
    if normalized.contains("GCM") {
        return match key.len() {
            16 => aes_gcm::<Aes128>(key, iv, input, encrypt),
            24 => aes_gcm::<Aes192>(key, iv, input, encrypt),
            32 => aes_gcm::<Aes256>(key, iv, input, encrypt),
            _ => Err("AES key must be 16, 24, or 32 bytes".to_owned()),
        };
    }
    let no_padding = normalized.contains("NOPADDING");
    let cbc = !normalized.contains("ECB");
    match key.len() {
        16 => aes_block_mode::<Aes128>(key, iv, input, encrypt, cbc, no_padding),
        24 => aes_block_mode::<Aes192>(key, iv, input, encrypt, cbc, no_padding),
        32 => aes_block_mode::<Aes256>(key, iv, input, encrypt, cbc, no_padding),
        _ => Err("AES key must be 16, 24, or 32 bytes".to_owned()),
    }
}

fn random_bytes(length: usize) -> CryptoResult<Vec<u8>> {
    if length > MAX_RANDOM_BYTES {
        return Err("Random byte request exceeds 65536 bytes".to_owned());
    }
    let mut output = vec![0u8; length];
    SystemRandom::new()
        .fill(&mut output)
        .map_err(|_| "Secure random generation failed".to_owned())?;
    Ok(output)
}

fn parse_hex(value: &str) -> Vec<u8> {
    let mut chars = value
        .chars()
        .filter(|character| character.is_ascii_hexdigit())
        .collect::<Vec<_>>();
    if chars.len() % 2 == 1 {
        chars.insert(0, '0');
    }
    chars
        .chunks_exact(2)
        .map(|pair| {
            let high = pair[0].to_digit(16).unwrap_or(0) as u8;
            let low = pair[1].to_digit(16).unwrap_or(0) as u8;
            (high << 4) | low
        })
        .collect()
}

fn hex(bytes: &[u8]) -> String {
    const TABLE: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(TABLE[(byte >> 4) as usize] as char);
        output.push(TABLE[(byte & 0x0f) as usize] as char);
    }
    output
}

fn decode_base64(value: &str) -> CryptoResult<Vec<u8>> {
    let mut normalized = value
        .trim()
        .replace(['\n', '\r', ' '], "")
        .replace('-', "+")
        .replace('_', "/");
    let padding = (4 - normalized.len() % 4) % 4;
    normalized.extend(std::iter::repeat_n('=', padding));
    STANDARD
        .decode(normalized.as_bytes())
        .map_err(|error| error.to_string())
}

fn encode_base64(value: &[u8]) -> String {
    STANDARD.encode(value)
}

fn read_der<'a>(bytes: &'a [u8], offset: &mut usize) -> CryptoResult<(u8, &'a [u8])> {
    let tag = *bytes
        .get(*offset)
        .ok_or_else(|| "Invalid DER key".to_owned())?;
    *offset += 1;
    let first_length = *bytes
        .get(*offset)
        .ok_or_else(|| "Invalid DER key".to_owned())?;
    *offset += 1;
    let length = if first_length & 0x80 == 0 {
        first_length as usize
    } else {
        let count = (first_length & 0x7f) as usize;
        if count == 0 || count > 4 || (*offset).saturating_add(count) > bytes.len() {
            return Err("Invalid DER length".to_owned());
        }
        let mut length = 0usize;
        for _ in 0..count {
            length = (length << 8)
                | (*bytes
                    .get(*offset)
                    .ok_or_else(|| "Invalid DER key".to_owned())? as usize);
            *offset += 1;
        }
        length
    };
    let end = (*offset)
        .checked_add(length)
        .filter(|end| *end <= bytes.len())
        .ok_or_else(|| "Invalid DER element length".to_owned())?;
    let value = &bytes[*offset..end];
    *offset = end;
    Ok((tag, value))
}

fn spki_key_bytes(der: &[u8]) -> CryptoResult<&[u8]> {
    let mut offset = 0;
    let (tag, outer) = read_der(der, &mut offset)?;
    if tag != 0x30 || offset != der.len() {
        return Err("Invalid X.509 public key".to_owned());
    }
    let mut cursor = 0;
    let (algorithm_tag, _) = read_der(outer, &mut cursor)?;
    if algorithm_tag != 0x30 {
        return Err("Invalid X.509 key algorithm".to_owned());
    }
    let (bit_string_tag, bit_string) = read_der(outer, &mut cursor)?;
    if bit_string_tag != 0x03 || bit_string.first() != Some(&0) {
        return Err("Invalid X.509 public key bit string".to_owned());
    }
    Ok(&bit_string[1..])
}

fn rsa_public_components(der: &[u8]) -> CryptoResult<(Vec<u8>, Vec<u8>)> {
    let key = spki_key_bytes(der)?;
    let mut offset = 0;
    let (tag, sequence) = read_der(key, &mut offset)?;
    if tag != 0x30 || offset != key.len() {
        return Err("Invalid RSA public key".to_owned());
    }
    let mut cursor = 0;
    let (modulus_tag, modulus) = read_der(sequence, &mut cursor)?;
    let (exponent_tag, exponent) = read_der(sequence, &mut cursor)?;
    if modulus_tag != 0x02 || exponent_tag != 0x02 {
        return Err("Invalid RSA public key integers".to_owned());
    }
    let modulus = modulus.strip_prefix(&[0]).unwrap_or(modulus).to_vec();
    let exponent = exponent.strip_prefix(&[0]).unwrap_or(exponent).to_vec();
    Ok((modulus, exponent))
}

fn signature_algorithm(name: &str) -> &'static str {
    if name.to_ascii_uppercase().contains("ECDSA") {
        "ECDSA"
    } else {
        "RSA"
    }
}

fn sign_bytes(algorithm: &str, private_key: &[u8], data: &[u8]) -> CryptoResult<Vec<u8>> {
    let rng = SystemRandom::new();
    match signature_algorithm(algorithm) {
        "ECDSA" => {
            let key = EcdsaKeyPair::from_pkcs8(
                &ECDSA_P256_SHA256_ASN1_SIGNING,
                private_key,
                &rng,
            )
            .map_err(|_| "Invalid ECDSA PKCS#8 private key".to_owned())?;
            key.sign(&rng, data)
                .map(|signature| signature.as_ref().to_vec())
                .map_err(|_| "ECDSA signing failed".to_owned())
        }
        _ => {
            let key = RsaKeyPair::from_pkcs8(private_key)
                .map_err(|_| "Invalid RSA PKCS#8 private key".to_owned())?;
            let mut output = vec![0u8; key.public().modulus_len()];
            key.sign(&RSA_PKCS1_SHA256, &rng, data, &mut output)
                .map_err(|_| "RSA signing failed".to_owned())?;
            Ok(output)
        }
    }
}

fn verify_bytes(
    algorithm: &str,
    public_key: &[u8],
    signature: &[u8],
    data: &[u8],
) -> CryptoResult<bool> {
    match signature_algorithm(algorithm) {
        "ECDSA" => {
            let key = spki_key_bytes(public_key)?;
            Ok(UnparsedPublicKey::new(&ECDSA_P256_SHA256_ASN1, key)
                .verify(data, signature)
                .is_ok())
        }
        _ => {
            let (modulus, exponent) = rsa_public_components(public_key)?;
            Ok(RsaPublicKeyComponents {
                n: &modulus,
                e: &exponent,
            }
            .verify(&RSA_PKCS1_2048_8192_SHA256, data, signature)
            .is_ok())
        }
    }
}

pub(crate) fn register_crypto_bridge(globals: &Object<'_>) -> rquickjs::Result<()> {
    globals.set(
        "__crypto_get_random_values_hex",
        Func::from(|length: i32| -> String {
            usize::try_from(length)
                .ok()
                .and_then(|length| random_bytes(length).ok())
                .map(|bytes| hex(&bytes))
                .unwrap_or_default()
        }),
    )?;
    globals.set(
        "__crypto_digest_hex_raw",
        Func::from(|algorithm: String, input: String| -> String {
            digest_bytes(&algorithm, &parse_hex(&input))
                .map(|bytes| hex(&bytes))
                .unwrap_or_default()
        }),
    )?;
    globals.set(
        "__crypto_hmac_hex_raw",
        Func::from(|algorithm: String, key: String, input: String| -> String {
            hmac_bytes(&algorithm, &parse_hex(&key), &parse_hex(&input))
                .map(|bytes| hex(&bytes))
                .unwrap_or_default()
        }),
    )?;
    globals.set(
        "__crypto_pbkdf2_hex",
        Func::from(
            |password: String, salt: String, iterations: i32, key_bits: i32, algorithm: String| -> String {
                let output_bytes = usize::try_from(key_bits)
                    .ok()
                    .filter(|bits| bits % 8 == 0)
                    .map(|bits| bits / 8);
                let result = output_bytes.and_then(|output_bytes| {
                    pbkdf2_bytes(
                        &algorithm,
                        &parse_hex(&password),
                        &parse_hex(&salt),
                        u32::try_from(iterations).ok()?,
                        output_bytes,
                    )
                    .ok()
                });
                result.map(|bytes| hex(&bytes)).unwrap_or_default()
            },
        ),
    )?;
    globals.set(
        "__crypto_aes_encrypt_hex",
        Func::from(|mode: String, key: String, iv: String, input: String| -> String {
            aes_bytes(
                &mode,
                &parse_hex(&key),
                &parse_hex(&iv),
                &parse_hex(&input),
                true,
            )
            .map(|bytes| hex(&bytes))
            .unwrap_or_default()
        }),
    )?;
    globals.set(
        "__crypto_aes_decrypt_hex",
        Func::from(|mode: String, key: String, iv: String, input: String| -> String {
            aes_bytes(
                &mode,
                &parse_hex(&key),
                &parse_hex(&iv),
                &parse_hex(&input),
                false,
            )
            .map(|bytes| hex(&bytes))
            .unwrap_or_default()
        }),
    )?;
    globals.set(
        "__crypto_sign_hex",
        Func::from(|algorithm: String, private_key: String, data: String| -> String {
            sign_bytes(&algorithm, &parse_hex(&private_key), &parse_hex(&data))
                .map(|bytes| hex(&bytes))
                .unwrap_or_default()
        }),
    )?;
    globals.set(
        "__crypto_verify_hex",
        Func::from(
            |algorithm: String, public_key: String, signature: String, data: String| -> bool {
                verify_bytes(
                    &algorithm,
                    &parse_hex(&public_key),
                    &parse_hex(&signature),
                    &parse_hex(&data),
                )
                .unwrap_or(false)
            },
        ),
    )?;

    // Legacy helpers retained for repositories written against the earlier host bridge.
    globals.set(
        "__crypto_digest_hex",
        Func::from(|algorithm: String, input: String| -> String {
            digest_bytes(&algorithm, input.as_bytes())
                .map(|bytes| hex(&bytes))
                .unwrap_or_default()
        }),
    )?;
    globals.set(
        "__crypto_hmac_hex",
        Func::from(|algorithm: String, key: String, input: String| -> String {
            hmac_bytes(&algorithm, key.as_bytes(), input.as_bytes())
                .map(|bytes| hex(&bytes))
                .unwrap_or_default()
        }),
    )?;
    globals.set(
        "__crypto_base64_encode",
        Func::from(|input: String| -> String { encode_base64(input.as_bytes()) }),
    )?;
    globals.set(
        "__crypto_base64_decode",
        Func::from(|input: String| -> String {
            decode_base64(&input)
                .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
                .unwrap_or_default()
        }),
    )?;
    globals.set(
        "__crypto_utf8_to_hex",
        Func::from(|input: String| -> String { hex(input.as_bytes()) }),
    )?;
    globals.set(
        "__crypto_hex_to_utf8",
        Func::from(|input: String| -> String {
            String::from_utf8_lossy(&parse_hex(&input)).into_owned()
        }),
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digest_and_hmac_vectors_match_standard_implementations() {
        assert_eq!(
            hex(&digest_bytes("SHA-256", b"abc").unwrap()),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            hex(&digest_bytes("MD5", b"abc").unwrap()),
            "900150983cd24fb0d6963f7d28e17f72"
        );
        assert_eq!(
            hex(&hmac_bytes(
                "SHA256",
                b"key",
                b"The quick brown fox jumps over the lazy dog"
            )
            .unwrap()),
            "f7bc83f430538424b13298e6aa6fb143ef4d59a14946175997479dbc2d1a3cd8"
        );
    }

    #[test]
    fn pbkdf2_matches_rfc_6070_sha1_vector() {
        assert_eq!(
            hex(&pbkdf2_bytes("SHA1", b"password", b"salt", 1, 20).unwrap()),
            "0c60c80f961f0e71f3a9b524af6012062fe037a6"
        );
    }

    #[test]
    fn aes_cbc_and_gcm_round_trip() {
        let key = [7u8; 32];
        let iv = [9u8; 16];
        let plaintext = b"SENZO native scraper crypto compatibility";
        let encrypted = aes_bytes("AES-CBC", &key, &iv, plaintext, true).unwrap();
        assert_ne!(encrypted, plaintext);
        assert_eq!(
            aes_bytes("AES-CBC", &key, &iv, &encrypted, false).unwrap(),
            plaintext
        );

        let nonce = [3u8; 12];
        let encrypted = aes_bytes("AES-GCM", &key, &nonce, plaintext, true).unwrap();
        assert_eq!(
            aes_bytes("AES-GCM", &key, &nonce, &encrypted, false).unwrap(),
            plaintext
        );
    }

    #[test]
    fn aes_gcm_matches_nist_vectors_for_all_aes_key_sizes() {
        let iv = [0u8; 12];
        let plaintext = [0u8; 16];
        for (key, expected) in [
            (
                vec![0u8; 16],
                "0388dace60b6a392f328c2b971b2fe78ab6e47d42cec13bdf53a67b21257bddf",
            ),
            (
                vec![0u8; 24],
                "98e7247c07f0fe411c267e4384b0f6002ff58d80033927ab8ef4d4587514f0fb",
            ),
            (
                vec![0u8; 32],
                "cea7403d4d606b6e074ec5d3baf39d18d0d1c8a799996bf0265b98b5d48ab919",
            ),
        ] {
            let encrypted = aes_bytes("AES-GCM", &key, &iv, &plaintext, true).unwrap();
            assert_eq!(hex(&encrypted), expected);
            assert_eq!(
                aes_bytes("AES-GCM", &key, &iv, &encrypted, false).unwrap(),
                plaintext
            );
        }
    }

    #[test]
    fn aes_gcm_supports_non_96_bit_nonces() {
        let key = [0u8; 24];
        let iv = [0u8; 16];
        let plaintext = [0u8; 16];
        let encrypted = aes_bytes("AES-GCM", &key, &iv, &plaintext, true).unwrap();
        assert_eq!(
            hex(&encrypted),
            "d910084572a722684c2b06a3e4c3eb2a5e45b7100696dcadbf835003e743e3d3"
        );
        assert_eq!(
            aes_bytes("AES-GCM", &key, &iv, &encrypted, false).unwrap(),
            plaintext
        );
    }

    #[test]
    fn rejects_unbounded_crypto_work() {
        assert!(pbkdf2_bytes("SHA256", b"p", b"s", MAX_PBKDF2_ITERATIONS + 1, 32).is_err());
        assert!(random_bytes(MAX_RANDOM_BYTES + 1).is_err());
    }
}
