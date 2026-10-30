//! The object identifiers OpenSSL names, as (dotted OID, short name, long
//! name, NID) from its objects table: the ones certificates use.

pub struct Obj {
    pub oid: &'static str,
    pub sn: &'static str,
    pub ln: &'static str,
    pub nid: i64,
}

macro_rules! o {
    ($oid:literal, $sn:literal, $ln:literal, $nid:literal) => {
        Obj { oid: $oid, sn: $sn, ln: $ln, nid: $nid }
    };
}

pub const OBJECTS: &[Obj] = &[
    // distinguished name attributes
    o!("2.5.4.3", "CN", "commonName", 13),
    o!("2.5.4.4", "SN", "surname", 100),
    o!("2.5.4.5", "serialNumber", "serialNumber", 105),
    o!("2.5.4.6", "C", "countryName", 14),
    o!("2.5.4.7", "L", "localityName", 15),
    o!("2.5.4.8", "ST", "stateOrProvinceName", 16),
    o!("2.5.4.9", "street", "streetAddress", 660),
    o!("2.5.4.10", "O", "organizationName", 17),
    o!("2.5.4.11", "OU", "organizationalUnitName", 18),
    o!("2.5.4.12", "title", "title", 106),
    o!("2.5.4.13", "description", "description", 107),
    o!("2.5.4.15", "businessCategory", "businessCategory", 860),
    o!("2.5.4.16", "postalAddress", "postalAddress", 861),
    o!("2.5.4.17", "postalCode", "postalCode", 661),
    o!("2.5.4.18", "postOfficeBox", "postOfficeBox", 862),
    o!("2.5.4.20", "telephoneNumber", "telephoneNumber", 864),
    o!("2.5.4.22", "teletexTerminalIdentifier", "teletexTerminalIdentifier", 866),
    o!("2.5.4.41", "name", "name", 173),
    o!("2.5.4.42", "GN", "givenName", 99),
    o!("2.5.4.43", "initials", "initials", 101),
    o!("2.5.4.44", "generationQualifier", "generationQualifier", 509),
    o!("2.5.4.45", "x500UniqueIdentifier", "x500UniqueIdentifier", 503),
    o!("2.5.4.46", "dnQualifier", "dnQualifier", 174),
    o!("2.5.4.65", "pseudonym", "pseudonym", 510),
    o!("2.5.4.72", "role", "role", 400),
    o!("2.5.4.97", "organizationIdentifier", "organizationIdentifier", 1089),
    o!("1.2.840.113549.1.9.1", "emailAddress", "emailAddress", 48),
    o!("1.2.840.113549.1.9.2", "unstructuredName", "unstructuredName", 49),
    o!("1.2.840.113549.1.9.8", "unstructuredAddress", "unstructuredAddress", 55),
    o!("0.9.2342.19200300.100.1.1", "UID", "userId", 458),
    o!("0.9.2342.19200300.100.1.25", "DC", "domainComponent", 391),
    o!("1.3.6.1.4.1.311.60.2.1.1", "jurisdictionL", "jurisdictionLocalityName", 955),
    o!("1.3.6.1.4.1.311.60.2.1.2", "jurisdictionST", "jurisdictionStateOrProvinceName", 956),
    o!("1.3.6.1.4.1.311.60.2.1.3", "jurisdictionC", "jurisdictionCountryName", 957),
    // signature algorithms
    o!("1.2.840.113549.1.1.1", "rsaEncryption", "rsaEncryption", 6),
    o!("1.2.840.113549.1.1.2", "RSA-MD2", "md2WithRSAEncryption", 7),
    o!("1.2.840.113549.1.1.3", "RSA-MD4", "md4WithRSAEncryption", 396),
    o!("1.2.840.113549.1.1.4", "RSA-MD5", "md5WithRSAEncryption", 8),
    o!("1.2.840.113549.1.1.5", "RSA-SHA1", "sha1WithRSAEncryption", 65),
    o!("1.2.840.113549.1.1.10", "RSASSA-PSS", "rsassaPss", 912),
    o!("1.2.840.113549.1.1.11", "RSA-SHA256", "sha256WithRSAEncryption", 668),
    o!("1.2.840.113549.1.1.12", "RSA-SHA384", "sha384WithRSAEncryption", 669),
    o!("1.2.840.113549.1.1.13", "RSA-SHA512", "sha512WithRSAEncryption", 670),
    o!("1.2.840.113549.1.1.14", "RSA-SHA224", "sha224WithRSAEncryption", 671),
    o!("1.3.14.3.2.29", "RSA-SHA1-2", "sha1WithRSA", 115),
    o!("1.2.840.10040.4.3", "DSA-SHA1", "dsaWithSHA1", 113),
    o!("2.16.840.1.101.3.4.3.1", "dsa_with_SHA224", "dsa_with_SHA224", 802),
    o!("2.16.840.1.101.3.4.3.2", "dsa_with_SHA256", "dsa_with_SHA256", 803),
    o!("1.2.840.10045.4.1", "ecdsa-with-SHA1", "ecdsa-with-SHA1", 416),
    o!("1.2.840.10045.4.3.1", "ecdsa-with-SHA224", "ecdsa-with-SHA224", 793),
    o!("1.2.840.10045.4.3.2", "ecdsa-with-SHA256", "ecdsa-with-SHA256", 794),
    o!("1.2.840.10045.4.3.3", "ecdsa-with-SHA384", "ecdsa-with-SHA384", 795),
    o!("1.2.840.10045.4.3.4", "ecdsa-with-SHA512", "ecdsa-with-SHA512", 796),
    o!("1.3.101.112", "ED25519", "ED25519", 1087),
    o!("1.3.101.113", "ED448", "ED448", 1088),
    // extensions
    o!("2.5.29.9", "subjectDirectoryAttributes", "X509v3 Subject Directory Attributes", 769),
    o!("2.5.29.14", "subjectKeyIdentifier", "X509v3 Subject Key Identifier", 82),
    o!("2.5.29.15", "keyUsage", "X509v3 Key Usage", 83),
    o!("2.5.29.16", "privateKeyUsagePeriod", "X509v3 Private Key Usage Period", 84),
    o!("2.5.29.17", "subjectAltName", "X509v3 Subject Alternative Name", 85),
    o!("2.5.29.18", "issuerAltName", "X509v3 Issuer Alternative Name", 86),
    o!("2.5.29.19", "basicConstraints", "X509v3 Basic Constraints", 87),
    o!("2.5.29.20", "crlNumber", "X509v3 CRL Number", 88),
    o!("2.5.29.30", "nameConstraints", "X509v3 Name Constraints", 666),
    o!("2.5.29.31", "crlDistributionPoints", "X509v3 CRL Distribution Points", 103),
    o!("2.5.29.32", "certificatePolicies", "X509v3 Certificate Policies", 89),
    o!("2.5.29.32.0", "anyPolicy", "X509v3 Any Policy", 746),
    o!("2.5.29.33", "policyMappings", "X509v3 Policy Mappings", 747),
    o!("2.5.29.35", "authorityKeyIdentifier", "X509v3 Authority Key Identifier", 90),
    o!("2.5.29.36", "policyConstraints", "X509v3 Policy Constraints", 401),
    o!("2.5.29.37", "extendedKeyUsage", "X509v3 Extended Key Usage", 126),
    o!("2.5.29.46", "freshestCRL", "X509v3 Freshest CRL", 857),
    o!("2.5.29.54", "inhibitAnyPolicy", "X509v3 Inhibit Any Policy", 748),
    o!("1.3.6.1.5.5.7.1.1", "authorityInfoAccess", "Authority Information Access", 177),
    o!("1.3.6.1.5.5.7.1.11", "subjectInfoAccess", "Subject Information Access", 398),
    o!("1.3.6.1.5.5.7.1.24", "tlsfeature", "TLS Feature", 1020),
    o!("1.3.6.1.4.1.11129.2.4.2", "ct_precert_scts", "CT Precertificate SCTs", 951),
    o!("1.3.6.1.4.1.11129.2.4.3", "ct_precert_poison", "CT Precertificate Poison", 952),
    o!("2.16.840.1.113730.1.1", "nsCertType", "Netscape Cert Type", 71),
    o!("2.16.840.1.113730.1.2", "nsBaseUrl", "Netscape Base Url", 72),
    o!("2.16.840.1.113730.1.3", "nsRevocationUrl", "Netscape Revocation Url", 73),
    o!("2.16.840.1.113730.1.4", "nsCaRevocationUrl", "Netscape CA Revocation Url", 74),
    o!("2.16.840.1.113730.1.7", "nsRenewalUrl", "Netscape Renewal Url", 75),
    o!("2.16.840.1.113730.1.8", "nsCaPolicyUrl", "Netscape CA Policy Url", 76),
    o!("2.16.840.1.113730.1.12", "nsSslServerName", "Netscape SSL Server Name", 77),
    o!("2.16.840.1.113730.1.13", "nsComment", "Netscape Comment", 78),
    // extended key usages and access methods
    o!("1.3.6.1.5.5.7.3.1", "serverAuth", "TLS Web Server Authentication", 129),
    o!("1.3.6.1.5.5.7.3.2", "clientAuth", "TLS Web Client Authentication", 130),
    o!("1.3.6.1.5.5.7.3.3", "codeSigning", "Code Signing", 131),
    o!("1.3.6.1.5.5.7.3.4", "emailProtection", "E-mail Protection", 132),
    o!("1.3.6.1.5.5.7.3.8", "timeStamping", "Time Stamping", 133),
    o!("1.3.6.1.5.5.7.3.9", "OCSPSigning", "OCSP Signing", 180),
    o!("1.3.6.1.5.5.7.3.17", "ipsecIKE", "ipsec Internet Key Exchange", 1022),
    o!("2.5.29.37.0", "anyExtendedKeyUsage", "Any Extended Key Usage", 910),
    o!("1.3.6.1.4.1.311.10.3.3", "msSGC", "Microsoft Server Gated Crypto", 137),
    o!("2.16.840.1.113730.4.1", "nsSGC", "Netscape Server Gated Crypto", 139),
    o!("1.3.6.1.4.1.311.2.1.21", "msCodeInd", "Microsoft Individual Code Signing", 134),
    o!("1.3.6.1.4.1.311.2.1.22", "msCodeCom", "Microsoft Commercial Code Signing", 135),
    o!("1.3.6.1.4.1.311.10.3.1", "msCTLSign", "Microsoft Trust List Signing", 136),
    o!("1.3.6.1.4.1.311.10.3.4", "msEFS", "Microsoft Encrypted File System", 138),
    o!("1.3.6.1.4.1.311.20.2.2", "msSmartcardLogin", "Microsoft Smartcard Login", 648),
    o!("1.3.6.1.5.5.7.48.1", "OCSP", "OCSP", 178),
    o!("1.3.6.1.5.5.7.48.2", "caIssuers", "CA Issuers", 179),
    o!("1.3.6.1.5.5.7.2.1", "id-qt-cps", "Policy Qualifier CPS", 164),
    o!("1.3.6.1.5.5.7.2.2", "id-qt-unotice", "Policy Qualifier User Notice", 165),
    // key types and curves
    o!("1.2.840.10045.2.1", "id-ecPublicKey", "id-ecPublicKey", 408),
    o!("1.2.840.10045.3.1.7", "prime256v1", "prime256v1", 415),
    o!("1.3.132.0.34", "secp384r1", "secp384r1", 715),
    o!("1.3.132.0.35", "secp521r1", "secp521r1", 716),
    o!("1.3.132.0.10", "secp256k1", "secp256k1", 714),
    o!("1.3.101.110", "X25519", "X25519", 1034),
];

pub fn by_oid(oid: &str) -> Option<&'static Obj> {
    OBJECTS.iter().find(|o| o.oid == oid)
}

/// OBJ_obj2txt with names: the long name, else the dotted form.
pub fn ln_or_dotted(oid: &str) -> String {
    by_oid(oid).map_or_else(|| oid.to_string(), |o| o.ln.to_string())
}
