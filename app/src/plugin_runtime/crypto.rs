//! RustCrypto primitives used by the provider compatibility layer.

use aes::cipher::{
    BlockDecryptMut, BlockEncryptMut, KeyInit, KeyIvInit,
    block_padding::{NoPadding, Pkcs7},
};
use aes_gcm::{
    Aes128Gcm, Aes256Gcm, Nonce,
    aead::Aead,
};
use hmac::{Hmac, Mac, digest::KeyInit as MacKeyInit};
use md5::Md5;
use pbkdf2::pbkdf2_hmac;
use rand::RngExt;
use sha1::Sha1;
use sha2::{Digest, Sha256, Sha384, Sha512};

type Aes128CbcEnc = cbc::Encryptor<aes::Aes128>;
type Aes128CbcDec = cbc::Decryptor<aes::Aes128>;
type Aes192CbcEnc = cbc::Encryptor<aes::Aes192>;
type Aes192CbcDec = cbc::Decryptor<aes::Aes192>;
type Aes256CbcEnc = cbc::Encryptor<aes::Aes256>;
type Aes256CbcDec = cbc::Decryptor<aes::Aes256>;
type Aes128EcbEnc = ecb::Encryptor<aes::Aes128>;
type Aes128EcbDec = ecb::Decryptor<aes::Aes128>;
type Aes192EcbEnc = ecb::Encryptor<aes::Aes192>;
type Aes192EcbDec = ecb::Decryptor<aes::Aes192>;
type Aes256EcbEnc = ecb::Encryptor<aes::Aes256>;
type Aes256EcbDec = ecb::Decryptor<aes::Aes256>;

pub(super) fn digest(name: &str, data: &[u8]) -> Result<Vec<u8>, String> {
    match normalize_hash(name) {
        "MD5" => Ok(Md5::digest(data).to_vec()),
        "SHA1" => Ok(Sha1::digest(data).to_vec()),
        "SHA256" => Ok(Sha256::digest(data).to_vec()),
        "SHA384" => Ok(Sha384::digest(data).to_vec()),
        "SHA512" => Ok(Sha512::digest(data).to_vec()),
        _ => Err("Unsupported hash algorithm.".to_owned()),
    }
}

pub(super) fn hmac(name: &str, key: &[u8], data: &[u8]) -> Result<Vec<u8>, String> {
    match normalize_hash(name) {
        "MD5" => mac::<Hmac<Md5>>(key, data),
        "SHA1" => mac::<Hmac<Sha1>>(key, data),
        "SHA256" => mac::<Hmac<Sha256>>(key, data),
        "SHA384" => mac::<Hmac<Sha384>>(key, data),
        "SHA512" => mac::<Hmac<Sha512>>(key, data),
        _ => Err("Unsupported HMAC algorithm.".to_owned()),
    }
}

fn mac<M>(key: &[u8], data: &[u8]) -> Result<Vec<u8>, String>
where
    M: Mac + MacKeyInit,
{
    let mut mac = <M as MacKeyInit>::new_from_slice(key)
        .map_err(|_| "Invalid HMAC key.".to_owned())?;
    mac.update(data);
    Ok(mac.finalize().into_bytes().to_vec())
}

pub(super) fn pbkdf2(
    password: &[u8],
    salt: &[u8],
    iterations: u32,
    bits: u32,
    hash: &str,
) -> Result<Vec<u8>, String> {
    if iterations == 0 || iterations > 1_000_000 {
        return Err("PBKDF2 iteration count is outside the allowed range.".to_owned());
    }
    if bits == 0 || bits > 16_384 || bits % 8 != 0 {
        return Err("PBKDF2 key size is outside the allowed range.".to_owned());
    }
    let mut output = vec![0u8; (bits / 8) as usize];
    match normalize_hash(hash) {
        "MD5" => pbkdf2_hmac::<Md5>(password, salt, iterations, &mut output),
        "SHA1" => pbkdf2_hmac::<Sha1>(password, salt, iterations, &mut output),
        "SHA256" => pbkdf2_hmac::<Sha256>(password, salt, iterations, &mut output),
        "SHA384" => pbkdf2_hmac::<Sha384>(password, salt, iterations, &mut output),
        "SHA512" => pbkdf2_hmac::<Sha512>(password, salt, iterations, &mut output),
        _ => return Err("Unsupported PBKDF2 hash.".to_owned()),
    }
    Ok(output)
}

pub(super) fn aes(
    encrypt: bool,
    mode: &str,
    key: &[u8],
    iv: &[u8],
    data: &[u8],
) -> Result<Vec<u8>, String> {
    let (mode, no_padding) = match mode.strip_suffix("-NoPadding") {
        Some(mode) => (mode, true),
        None => (mode, false),
    };
    match mode {
        "AES-CBC" => aes_cbc(encrypt, key, iv, data, no_padding),
        "AES-ECB" => aes_ecb(encrypt, key, data, no_padding),
        "AES-GCM" => aes_gcm(encrypt, key, iv, data),
        _ => Err("Unsupported AES mode.".to_owned()),
    }
}

fn aes_cbc(encrypt: bool, key: &[u8], iv: &[u8], data: &[u8], no_padding: bool) -> Result<Vec<u8>, String> {
    if iv.len() != 16 {
        return Err("AES-CBC requires a 16-byte IV.".to_owned());
    }
    let result = match (key.len(), encrypt, no_padding) {
        (16, true, false) => Aes128CbcEnc::new_from_slices(key, iv)
            .map_err(|_| "Invalid AES key.".to_owned())?
            .encrypt_padded_vec_mut::<Pkcs7>(data),
        (16, true, true) => Aes128CbcEnc::new_from_slices(key, iv)
            .map_err(|_| "Invalid AES key.".to_owned())?
            .encrypt_padded_vec_mut::<NoPadding>(data),
        (16, false, false) => Aes128CbcDec::new_from_slices(key, iv)
            .map_err(|_| "Invalid AES key.".to_owned())?
            .decrypt_padded_vec_mut::<Pkcs7>(data)
            .map_err(|_| "AES-CBC padding was invalid.".to_owned())?,
        (16, false, true) => Aes128CbcDec::new_from_slices(key, iv)
            .map_err(|_| "Invalid AES key.".to_owned())?
            .decrypt_padded_vec_mut::<NoPadding>(data)
            .map_err(|_| "AES-CBC input was not block aligned.".to_owned())?,
        (24, true, false) => Aes192CbcEnc::new_from_slices(key, iv)
            .map_err(|_| "Invalid AES key.".to_owned())?
            .encrypt_padded_vec_mut::<Pkcs7>(data),
        (24, true, true) => Aes192CbcEnc::new_from_slices(key, iv)
            .map_err(|_| "Invalid AES key.".to_owned())?
            .encrypt_padded_vec_mut::<NoPadding>(data),
        (24, false, false) => Aes192CbcDec::new_from_slices(key, iv)
            .map_err(|_| "Invalid AES key.".to_owned())?
            .decrypt_padded_vec_mut::<Pkcs7>(data)
            .map_err(|_| "AES-CBC padding was invalid.".to_owned())?,
        (24, false, true) => Aes192CbcDec::new_from_slices(key, iv)
            .map_err(|_| "Invalid AES key.".to_owned())?
            .decrypt_padded_vec_mut::<NoPadding>(data)
            .map_err(|_| "AES-CBC input was not block aligned.".to_owned())?,
        (32, true, false) => Aes256CbcEnc::new_from_slices(key, iv)
            .map_err(|_| "Invalid AES key.".to_owned())?
            .encrypt_padded_vec_mut::<Pkcs7>(data),
        (32, true, true) => Aes256CbcEnc::new_from_slices(key, iv)
            .map_err(|_| "Invalid AES key.".to_owned())?
            .encrypt_padded_vec_mut::<NoPadding>(data),
        (32, false, false) => Aes256CbcDec::new_from_slices(key, iv)
            .map_err(|_| "Invalid AES key.".to_owned())?
            .decrypt_padded_vec_mut::<Pkcs7>(data)
            .map_err(|_| "AES-CBC padding was invalid.".to_owned())?,
        (32, false, true) => Aes256CbcDec::new_from_slices(key, iv)
            .map_err(|_| "Invalid AES key.".to_owned())?
            .decrypt_padded_vec_mut::<NoPadding>(data)
            .map_err(|_| "AES-CBC input was not block aligned.".to_owned())?,
        _ => return Err("AES keys must be 16, 24, or 32 bytes.".to_owned()),
    };
    Ok(result)
}

fn aes_ecb(encrypt: bool, key: &[u8], data: &[u8], no_padding: bool) -> Result<Vec<u8>, String> {
    let result = match (key.len(), encrypt, no_padding) {
        (16, true, false) => Aes128EcbEnc::new_from_slice(key)
            .map_err(|_| "Invalid AES key.".to_owned())?
            .encrypt_padded_vec_mut::<Pkcs7>(data),
        (16, true, true) => Aes128EcbEnc::new_from_slice(key)
            .map_err(|_| "Invalid AES key.".to_owned())?
            .encrypt_padded_vec_mut::<NoPadding>(data),
        (16, false, false) => Aes128EcbDec::new_from_slice(key)
            .map_err(|_| "Invalid AES key.".to_owned())?
            .decrypt_padded_vec_mut::<Pkcs7>(data)
            .map_err(|_| "AES-ECB padding was invalid.".to_owned())?,
        (16, false, true) => Aes128EcbDec::new_from_slice(key)
            .map_err(|_| "Invalid AES key.".to_owned())?
            .decrypt_padded_vec_mut::<NoPadding>(data)
            .map_err(|_| "AES-ECB input was not block aligned.".to_owned())?,
        (24, true, false) => Aes192EcbEnc::new_from_slice(key)
            .map_err(|_| "Invalid AES key.".to_owned())?
            .encrypt_padded_vec_mut::<Pkcs7>(data),
        (24, true, true) => Aes192EcbEnc::new_from_slice(key)
            .map_err(|_| "Invalid AES key.".to_owned())?
            .encrypt_padded_vec_mut::<NoPadding>(data),
        (24, false, false) => Aes192EcbDec::new_from_slice(key)
            .map_err(|_| "Invalid AES key.".to_owned())?
            .decrypt_padded_vec_mut::<Pkcs7>(data)
            .map_err(|_| "AES-ECB padding was invalid.".to_owned())?,
        (24, false, true) => Aes192EcbDec::new_from_slice(key)
            .map_err(|_| "Invalid AES key.".to_owned())?
            .decrypt_padded_vec_mut::<NoPadding>(data)
            .map_err(|_| "AES-ECB input was not block aligned.".to_owned())?,
        (32, true, false) => Aes256EcbEnc::new_from_slice(key)
            .map_err(|_| "Invalid AES key.".to_owned())?
            .encrypt_padded_vec_mut::<Pkcs7>(data),
        (32, true, true) => Aes256EcbEnc::new_from_slice(key)
            .map_err(|_| "Invalid AES key.".to_owned())?
            .encrypt_padded_vec_mut::<NoPadding>(data),
        (32, false, false) => Aes256EcbDec::new_from_slice(key)
            .map_err(|_| "Invalid AES key.".to_owned())?
            .decrypt_padded_vec_mut::<Pkcs7>(data)
            .map_err(|_| "AES-ECB padding was invalid.".to_owned())?,
        (32, false, true) => Aes256EcbDec::new_from_slice(key)
            .map_err(|_| "Invalid AES key.".to_owned())?
            .decrypt_padded_vec_mut::<NoPadding>(data)
            .map_err(|_| "AES-ECB input was not block aligned.".to_owned())?,
        _ => return Err("AES keys must be 16, 24, or 32 bytes.".to_owned()),
    };
    Ok(result)
}

fn aes_gcm(encrypt: bool, key: &[u8], iv: &[u8], data: &[u8]) -> Result<Vec<u8>, String> {
    if iv.len() != 12 {
        return Err("AES-GCM currently requires a 12-byte nonce.".to_owned());
    }
    let nonce = Nonce::from_slice(iv);
    let result = match key.len() {
        16 => {
            let cipher = Aes128Gcm::new_from_slice(key).map_err(|_| "Invalid AES key.".to_owned())?;
            if encrypt {
                cipher.encrypt(nonce, data).map_err(|_| "AES-GCM encryption failed.".to_owned())?
            } else {
                cipher.decrypt(nonce, data).map_err(|_| "AES-GCM authentication failed.".to_owned())?
            }
        }
        32 => {
            let cipher = Aes256Gcm::new_from_slice(key).map_err(|_| "Invalid AES key.".to_owned())?;
            if encrypt {
                cipher.encrypt(nonce, data).map_err(|_| "AES-GCM encryption failed.".to_owned())?
            } else {
                cipher.decrypt(nonce, data).map_err(|_| "AES-GCM authentication failed.".to_owned())?
            }
        }
        _ => return Err("AES-GCM keys must be 16 or 32 bytes.".to_owned()),
    };
    Ok(result)
}

pub(super) fn random_bytes(length: usize) -> Vec<u8> {
    let mut bytes = vec![0u8; length.min(1024 * 1024)];
    rand::rng().fill(&mut bytes[..]);
    bytes
}

fn normalize_hash(name: &str) -> &'static str {
    match name
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .collect::<String>()
        .to_ascii_uppercase()
        .as_str()
    {
        "MD5" => "MD5",
        "SHA1" => "SHA1",
        "SHA256" => "SHA256",
        "SHA384" => "SHA384",
        "SHA512" => "SHA512",
        _ => "",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_matches_the_known_vector() {
        let value = digest("SHA256", b"abc").unwrap();
        assert_eq!(
            hex::encode(value),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn hmac_sha256_matches_rfc4231() {
        let value = hmac("SHA256", b"key", b"The quick brown fox jumps over the lazy dog").unwrap();
        assert_eq!(
            hex::encode(value),
            "f7bc83f430538424b13298e6aa6fb143ef4d59a14946175997479dbc2d1a3cd8"
        );
    }

    #[test]
    fn aes_cbc_round_trip() {
        let key = [7u8; 16];
        let iv = [9u8; 16];
        let cipher = aes(true, "AES-CBC", &key, &iv, b"senzo plugin").unwrap();
        let plain = aes(false, "AES-CBC", &key, &iv, &cipher).unwrap();
        assert_eq!(plain, b"senzo plugin");
    }

    #[test]
    fn aes_gcm_round_trip() {
        let key = [3u8; 16];
        let iv = [4u8; 12];
        let cipher = aes(true, "AES-GCM", &key, &iv, b"hello").unwrap();
        let plain = aes(false, "AES-GCM", &key, &iv, &cipher).unwrap();
        assert_eq!(plain, b"hello");
    }
}
