//! Windows certificate trust: chain evaluation against the Windows stores,
//! trusting a certificate through the current user's Root store (Windows asks
//! the user to confirm), and adding a digital ID to the Personal store.
//!
//! This file depends only on `anyhow` and `windows-sys`, so it can be
//! type-checked for Windows on its own.
use anyhow::{Result, bail, ensure};
use std::mem::{size_of, zeroed};
use std::ptr::{null, null_mut};
use windows_sys::Win32::Security::Cryptography::*;

const ENCODING: CERT_QUERY_ENCODING_TYPE = X509_ASN_ENCODING | PKCS_7_ASN_ENCODING;

/// A certificate context that frees itself.
struct Context(*mut CERT_CONTEXT);
impl Context {
    fn new(der: &[u8]) -> Option<Self> {
        let context =
            unsafe { CertCreateCertificateContext(ENCODING, der.as_ptr(), der.len() as u32) };
        (!context.is_null()).then_some(Self(context))
    }
}
impl Drop for Context {
    fn drop(&mut self) {
        unsafe { CertFreeCertificateContext(self.0) };
    }
}

/// A certificate store that closes itself.
struct Store(HCERTSTORE);
impl Store {
    fn memory() -> Option<Self> {
        let store = unsafe { CertOpenStore(CERT_STORE_PROV_MEMORY, 0, 0, 0, null()) };
        (!store.is_null()).then_some(Self(store))
    }
    /// A system store of the current user, such as "Root" or "My".
    fn current_user(name: &str) -> Result<Self> {
        let wide: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
        let store = unsafe {
            CertOpenStore(
                CERT_STORE_PROV_SYSTEM_W,
                0,
                0,
                CERT_SYSTEM_STORE_CURRENT_USER,
                wide.as_ptr().cast(),
            )
        };
        ensure!(
            !store.is_null(),
            "Windows could not open the {name} certificate store"
        );
        Ok(Self(store))
    }
}
impl Drop for Store {
    fn drop(&mut self) {
        unsafe { CertCloseStore(self.0, 0) };
    }
}

/// Whether Windows trusts this chain (signer first, then intermediates),
/// using only cached revocation data.
pub fn evaluate(chain: &[Vec<u8>]) -> bool {
    let Some((leaf, rest)) = chain.split_first() else {
        return false;
    };
    let (Some(leaf), Some(extra)) = (Context::new(leaf), Store::memory()) else {
        return false;
    };
    for der in rest {
        unsafe {
            CertAddEncodedCertificateToStore(
                extra.0,
                ENCODING,
                der.as_ptr(),
                der.len() as u32,
                CERT_STORE_ADD_ALWAYS,
                null_mut(),
            )
        };
    }
    unsafe {
        let mut para: CERT_CHAIN_PARA = zeroed();
        para.cbSize = size_of::<CERT_CHAIN_PARA>() as u32;
        let mut context: *mut CERT_CHAIN_CONTEXT = null_mut();
        if CertGetCertificateChain(
            null_mut(),
            leaf.0,
            null(),
            extra.0,
            &para,
            CERT_CHAIN_CACHE_ONLY_URL_RETRIEVAL,
            null(),
            &mut context,
        ) == 0
            || context.is_null()
        {
            return false;
        }
        let mut policy: CERT_CHAIN_POLICY_PARA = zeroed();
        policy.cbSize = size_of::<CERT_CHAIN_POLICY_PARA>() as u32;
        let mut status: CERT_CHAIN_POLICY_STATUS = zeroed();
        status.cbSize = size_of::<CERT_CHAIN_POLICY_STATUS>() as u32;
        let verified =
            CertVerifyCertificateChainPolicy(CERT_CHAIN_POLICY_BASE, context, &policy, &mut status);
        CertFreeCertificateChain(context);
        verified != 0 && status.dwError == 0
    }
}

/// Trust a certificate by adding it to the current user's Root store.
/// Windows shows a security warning that the user must accept.
pub fn trust(der: &[u8]) -> Result<()> {
    let root = Store::current_user("Root")?;
    let added = unsafe {
        CertAddEncodedCertificateToStore(
            root.0,
            ENCODING,
            der.as_ptr(),
            der.len() as u32,
            CERT_STORE_ADD_REPLACE_EXISTING,
            null_mut(),
        )
    };
    ensure!(
        added != 0,
        "Windows did not add the certificate to your trusted roots"
    );
    Ok(())
}

/// Remove a certificate from the current user's Root store.
pub fn untrust(der: &[u8]) -> Result<()> {
    let root = Store::current_user("Root")?;
    let Some(wanted) = Context::new(der) else {
        bail!("Not a valid certificate");
    };
    let found = unsafe {
        CertFindCertificateInStore(
            root.0,
            ENCODING,
            0,
            CERT_FIND_EXISTING,
            wanted.0.cast(),
            null(),
        )
    };
    ensure!(
        !found.is_null(),
        "The certificate is not in your trusted roots"
    );
    // CertDeleteCertificateFromStore frees the found context.
    let deleted = unsafe { CertDeleteCertificateFromStore(found) };
    ensure!(
        deleted != 0,
        "Windows did not remove the certificate from your trusted roots"
    );
    Ok(())
}

/// Add a PKCS#12 digital ID (certificate and private key) to the current
/// user's Personal store.
pub fn import_identity(pkcs12: &[u8], password: &str) -> Result<()> {
    let blob = CRYPT_INTEGER_BLOB {
        cbData: pkcs12.len() as u32,
        pbData: pkcs12.as_ptr() as *mut u8,
    };
    let wide: Vec<u16> = password.encode_utf16().chain(Some(0)).collect();
    let imported = unsafe { PFXImportCertStore(&blob, wide.as_ptr(), CRYPT_USER_KEYSET) };
    ensure!(!imported.is_null(), "Windows could not read the digital ID");
    let imported = Store(imported);
    let personal = Store::current_user("My")?;
    let mut count = 0;
    let mut current: *mut CERT_CONTEXT = null_mut();
    loop {
        current = unsafe { CertEnumCertificatesInStore(imported.0, current) };
        if current.is_null() {
            break;
        }
        let added = unsafe {
            CertAddCertificateContextToStore(
                personal.0,
                current,
                CERT_STORE_ADD_REPLACE_EXISTING,
                null_mut(),
            )
        };
        count += (added != 0) as usize;
    }
    ensure!(
        count > 0,
        "The digital ID was not added to your Personal store"
    );
    Ok(())
}
