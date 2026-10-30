<?php
// Keys: reading every PEM form, details, export, signatures, RSA encryption.
$rsa = '-----BEGIN RSA PRIVATE KEY-----
MIICXAIBAAKBgQDLXp6PkCtbpV+P1gwFQWH6Ez0U83uEmS8IGnpeI8Fk8rY/vHOZ
zZZaxRCw+loyc342qCDIQheMOCNm5Fkevz06q757/oooiLR3yryYGKiKG1IZIipl
mtsC95oKrzUSKk60wuI1mbgpMUP5LKi/Tvxes5PmkUtXfimz2qgkeUcPpQIDAQAB
AoGBAMcP/dp+fsI9FFYBaVC3mASlUjOwxKWdH3kqGb8N9p4uKRAoEWtp3hNJM7ZX
x3P8sn0jgrsiXlRFGvn65/T9shp8hj+CdJKg2jKCs7S58v60TLfSvOQSIYsw9Qm9
Bsx4hKfz+d52ptuJRbv8tDxsYP3D/KjQfpX1OysiP/WBfeg9AkEA+AGT0goqjWOM
YgFtZGrefIegF31XSCQTaLIml6/2JwF+oBKjJUQFar2Rwn6qUwrsGtSPMM0Iz8ry
9uvUbs8PPwJBANHsuTVWzLf8TJNGc+xIlhvzKFkF0nJIWx4ozhlMNDQMMF/3FRSo
zvHIgUnpG9Vwa2GtjTDnD8jHtzTauAZmjBsCQCGDVQ5VAVsJ0LaNqtKe/mGlkiSa
c2j0Nws2x7BHvuOWeB35ZsJqZrD93OyDYVDHcRBPGOpnSoGJ0zs6swImSNECQHSH
0BgH4wSPDYMDrP4RHSLOzCr+zF+cQthvFll8r83kpkXfRth9DMOy5fI9cLH/Adzr
FmF7Iov2MYEpmNYUvtkCQHfW0ntkVY9xS2/VTs57F5tUkfNG2hG74pJM6vSfTNWn
R/oI5m2sDtRWQ88LCYJMEmIZhN00Ys4xOSoTs+SUakY=
-----END RSA PRIVATE KEY-----
';
$rsa_pub = '-----BEGIN PUBLIC KEY-----
MIGfMA0GCSqGSIb3DQEBAQUAA4GNADCBiQKBgQDLXp6PkCtbpV+P1gwFQWH6Ez0U
83uEmS8IGnpeI8Fk8rY/vHOZzZZaxRCw+loyc342qCDIQheMOCNm5Fkevz06q757
/oooiLR3yryYGKiKG1IZIiplmtsC95oKrzUSKk60wuI1mbgpMUP5LKi/Tvxes5Pm
kUtXfimz2qgkeUcPpQIDAQAB
-----END PUBLIC KEY-----
';
$rsa_enc = '-----BEGIN RSA PRIVATE KEY-----
Proc-Type: 4,ENCRYPTED
DEK-Info: AES-128-CBC,D839E12F98A22F4FA90401E31A896A03

gVJ/972mKRrZ2+55rofbFXuLY03Umb7+m7OotGHkYa3dJpNuOLta+LDpSMRdgKcT
7s+vZ7CERJsWolbdCSphwWs74FV4fJheuzrnEthmVazmIcDwWu1FiiD5XEmRQMFH
AvwUKTN5PIyN67eCusbrV5/FJsVdxvRwLUM70UE+nhBW8rg6AhszDtSHhGG1KN9T
TwT2m8xYOgzzPPseQN/VcKNJ+LZcUVN/8+12duy1Ej/HoFaE7wp5GH0mueeShf0R
xFZ6mizZ3x+Ef4+PbTDnSXRheF9uvSdwmff4rUS4Szpk0aYpSrluJJjzI9Unjvqi
129ZQsNUfLebJlaauTYYNsHHh64hRXelqKz5mNmcF/OV+pbCjYljgDsyndmrZ5jB
xjfjp/1VGQAU26VnF0X9zv5q9Gz3/bkl3VFnS7gNyMCanyTuaXNNFNcbkY59Iq1l
rWjGGMeujWttFhYsChtCb6Clsq4aCiXJ9lqFNTU+eqVD9F5C4v5ATM6VHqjxwvj3
RB4P98pQsGLkHUKgAZrYqxhLjN6O7FvZuuSRSZl+Q6suYcie+hR4p2TRopRTttc2
d/gdJCBEbboJH3XOFlHz08mhxgVobi5ACzAoSpCVT4NIahG+Gt7h8Rk6+A9/uiwj
Jy6+MRiylhAOXAfOps7+FvBFi4nKByFXtWOHRL6DGpYWy20vCk0gsuICRpwL33A4
e6YAJiuygQSWTExyGfeRk5VQRkZrWL1s3nTnA0MLqx1VdIbDeNPtIZ+dQHCN5vYt
pP7zFSuF6fse8mSobzA9x4hKXUKWQ/V985arHYzYpJXfIRMyZDqKjRW5xKGYR0eH
adhpbOCWx73+vVa8tABbvNIPjp/7KPaGPSp7uV9Afvvt5qjM/nho/VstyWscivM0
c+oylG4cNBaJCVyb6nsiBYK2XE/Llfjsj/l2+SRfIZOwmvhL+CG7z7okPEiNp7EB
DcefH7/N/Fg4R4PzWskHfb6ZU+nnPKV9JJdBitw/EQFAN8qO7b6O1HRsXUJziBsH
cgQBDT0wOue49lDFratmWBx1K10paRuAl1DmO5WtstC/cthGYvKVGDWclLbIVRpv
/DyXZMsXdt9IaP31vHs+W0Yj/1Pcj3iKFX/oh0e7Kb2MQziquQcMzJM2n3BVa2PQ
dZ3FGKSoDtdT3RhEWGAY5zh7nwPUDqR2frCTGdRAASJbHmC/dn3Fs/m7kQHCUTw8
EpDCCFZhp23DODk96Q0vtgtPU16qONMzVlzSH8REjS23GrbZ4wn/SW8Qm5QZiScH
WbJ3Aj5iemcFnL5aDN81BNX9GtUvJFK2jvmmPcv1x5+x51RweMQYErgahI9YVFtM
0cEN2cm27hZjktHxduJefc+WVu6vCyzSA0q/CbT58eIWBPyvyIM8AB65h4TFxX9N
4g0/0Kh8gqe3EVg1x62LEAC6AaRZEm/GBeux41DlNDYbCik6VdLRHVozmf7PYm+X
6miR4EfGE+kM7/V+shMQZni/Q64gzIn+/vpMiNV5JQsr1E2tqUp2/P6N0puUwWv2
ORVWWk6gy8wnJ//LsDNEhSKfCrffuv2u2oVQWMcBwyqbR6ru/V3i9+9ngWd2D4E0
-----END RSA PRIVATE KEY-----
';
$ec = '-----BEGIN EC PRIVATE KEY-----
MHcCAQEEILPkqoeyM7XgwYkuSj3077lrsrfWJK5LqMolv+m2oOjZoAoGCCqGSM49
AwEHoUQDQgAEPq4hbIWHvB51rdWr8ejrjWo4qVNWVugYFtPg/xLQw0mHkIPZ4DvK
sqOTOnMoezkbSmVVMuwz9flvnqHGmQvmug==
-----END EC PRIVATE KEY-----
';
$ec_pub = '-----BEGIN PUBLIC KEY-----
MFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAEPq4hbIWHvB51rdWr8ejrjWo4qVNW
VugYFtPg/xLQw0mHkIPZ4DvKsqOTOnMoezkbSmVVMuwz9flvnqHGmQvmug==
-----END PUBLIC KEY-----
';
$cert = '-----BEGIN CERTIFICATE-----
MIIDbDCCAtWgAwIBAgIJAK7FVsxyN1CiMA0GCSqGSIb3DQEBBQUAMIGBMQswCQYD
VQQGEwJCUjEaMBgGA1UECBMRUmlvIEdyYW5kZSBkbyBTdWwxFTATBgNVBAcTDFBv
cnRvIEFsZWdyZTEeMBwGA1UEAxMVSGVucmlxdWUgZG8gTi4gQW5nZWxvMR8wHQYJ
KoZIhvcNAQkBFhBobmFuZ2Vsb0BwaHAubmV0MB4XDTA4MDYzMDEwMjg0M1oXDTA4
MDczMDEwMjg0M1owgYExCzAJBgNVBAYTAkJSMRowGAYDVQQIExFSaW8gR3JhbmRl
IGRvIFN1bDEVMBMGA1UEBxMMUG9ydG8gQWxlZ3JlMR4wHAYDVQQDExVIZW5yaXF1
ZSBkbyBOLiBBbmdlbG8xHzAdBgkqhkiG9w0BCQEWEGhuYW5nZWxvQHBocC5uZXQw
gZ8wDQYJKoZIhvcNAQEBBQADgY0AMIGJAoGBAMteno+QK1ulX4/WDAVBYfoTPRTz
e4SZLwgael4jwWTytj+8c5nNllrFELD6WjJzfjaoIMhCF4w4I2bkWR6/PTqrvnv+
iiiItHfKvJgYqIobUhkiKmWa2wL3mgqvNRIqTrTC4jWZuCkxQ/ksqL9O/F6zk+aR
S1d+KbPaqCR5Rw+lAgMBAAGjgekwgeYwHQYDVR0OBBYEFNt+QHK9XDWF7CkpgRLo
Ymhqtz99MIG2BgNVHSMEga4wgauAFNt+QHK9XDWF7CkpgRLoYmhqtz99oYGHpIGE
MIGBMQswCQYDVQQGEwJCUjEaMBgGA1UECBMRUmlvIEdyYW5kZSBkbyBTdWwxFTAT
BgNVBAcTDFBvcnRvIEFsZWdyZTEeMBwGA1UEAxMVSGVucmlxdWUgZG8gTi4gQW5n
ZWxvMR8wHQYJKoZIhvcNAQkBFhBobmFuZ2Vsb0BwaHAubmV0ggkArsVWzHI3UKIw
DAYDVR0TBAUwAwEB/zANBgkqhkiG9w0BAQUFAAOBgQCP1GUnStC0TBqngr3Kx+zS
UW8KutKO0ORc5R8aV/x9LlaJrzPyQJgiPpu5hXogLSKRIHxQS3X2+Y0VvIpW72LW
PVKPhYlNtO3oKnfoJGKin0eEhXRZMjfEW/kznY+ZZmNifV2r8s+KhNAqI4PbClvn
4vh8xF/9+eVEj+hM+0OflA==
-----END CERTIFICATE-----
';

function show($d) {
    foreach ($d as $k => &$v) {
        if (is_array($v)) {
            $v = array_map(fn($x) => is_string($x) && !ctype_print($x) ? bin2hex($x) : $x, $v);
        }
    }
    print_r($d);
}

$k = openssl_pkey_get_private($rsa);
var_dump($k);
show(openssl_pkey_get_details($k));
var_dump(openssl_pkey_export($k, $out), $out);
show(openssl_pkey_get_details(openssl_pkey_get_public($rsa_pub)));
show(openssl_pkey_get_details(openssl_pkey_get_private($ec)));
show(openssl_pkey_get_details(openssl_pkey_get_public($ec_pub)));
var_dump(openssl_pkey_export(openssl_pkey_get_private($ec), $out), $out);
// a certificate's public key
show(openssl_pkey_get_details(openssl_get_publickey($cert)));
var_dump(openssl_pkey_get_details(openssl_pkey_get_public(openssl_x509_read($cert)))['bits']);
// encrypted keys: legacy PEM and PKCS#8
var_dump(openssl_pkey_get_private($rsa_enc));
var_dump(openssl_pkey_get_private($rsa_enc, "wrong"));
$k2 = openssl_pkey_get_private($rsa_enc, "php");
var_dump(openssl_pkey_get_details($k2)['bits']);
var_dump(openssl_pkey_get_private([$rsa_enc, "php"]) instanceof OpenSSLAsymmetricKey);
openssl_pkey_export($k, $enc, "pass phrase");
var_dump(strtok($enc, "\n"));
var_dump(openssl_pkey_get_private($enc, "nope"));
$back = openssl_pkey_get_private($enc, "pass phrase");
var_dump(openssl_pkey_get_details($back)['key'] === openssl_pkey_get_details($k)['key']);
openssl_pkey_export($k, $enc, "pp", ['encrypt_key_cipher' => OPENSSL_CIPHER_AES_256_CBC]);
var_dump(openssl_pkey_get_details(openssl_pkey_get_private($enc, "pp"))['bits']);
while (openssl_error_string());
// bad keys
var_dump(openssl_pkey_get_private("garbage"));
var_dump(openssl_error_string());
var_dump(openssl_pkey_get_public("garbage"));
var_dump(openssl_get_publickey($rsa_pub) instanceof OpenSSLAsymmetricKey, openssl_error_string());
try {
    openssl_pkey_get_private([$rsa]);
} catch (ValueError $e) {
    echo $e->getMessage(), "\n";
}
// signatures (RSA PKCS#1 v1.5 is deterministic)
$data = "Testing openssl_sign()";
foreach ([OPENSSL_ALGO_SHA1, OPENSSL_ALGO_SHA256, OPENSSL_ALGO_SHA512, OPENSSL_ALGO_MD5, "sha384", "RSA-SHA224"] as $algo) {
    var_dump(openssl_sign($data, $sig, $rsa, $algo));
    echo base64_encode($sig), "\n";
    var_dump(openssl_verify($data, $sig, $rsa_pub, $algo), openssl_verify($data . "x", $sig, $rsa_pub, $algo));
}
var_dump(openssl_sign($data, $sig, $k));
var_dump(openssl_verify($data, $sig, $rsa_pub));
var_dump(openssl_verify($data, $sig, openssl_pkey_get_public($rsa_pub)));
var_dump(openssl_verify($data, $sig, $k));
var_dump(openssl_sign($data, $sig, "wrong"));
var_dump(openssl_sign($data, $sig, $rsa, "nope"));
var_dump(openssl_verify($data, "wrong", $rsa_pub));
var_dump(openssl_sign($data, $sig, $ec, OPENSSL_ALGO_SHA256), openssl_verify($data, $sig, $ec_pub, OPENSSL_ALGO_SHA256), openssl_verify("other", $sig, $ec_pub, OPENSSL_ALGO_SHA256));
var_dump(openssl_sign($data, $sig, $ec, "sha1"), openssl_verify($data, $sig, $ec_pub, "sha1"));
// RSA encryption with each padding
foreach ([OPENSSL_PKCS1_PADDING, OPENSSL_PKCS1_OAEP_PADDING] as $pad) {
    var_dump(openssl_public_encrypt("secret message", $ct, $rsa_pub, $pad), strlen($ct));
    var_dump(openssl_private_decrypt($ct, $pt, $rsa, $pad), $pt);
}
var_dump(openssl_private_encrypt("signed", $ct, $rsa));
echo base64_encode($ct), "\n";
var_dump(openssl_public_decrypt($ct, $pt, $rsa_pub), $pt);
var_dump(openssl_public_encrypt(str_repeat("x", 200), $ct, $rsa_pub));
var_dump(openssl_error_string());
var_dump(openssl_public_encrypt("x", $ct, "nope"));
var_dump(openssl_private_decrypt("x", $ct, $rsa_pub));
// sealing
var_dump(openssl_seal("sealed data", $sealed, $ekeys, [$rsa_pub, openssl_pkey_get_public($cert)], "aes-256-cbc", $iv));
var_dump(strlen($sealed), count($ekeys), strlen($iv));
var_dump(openssl_open($sealed, $opened, $ekeys[0], $rsa, "aes-256-cbc", $iv), $opened);
// generated keys
$new = openssl_pkey_new(['private_key_bits' => 1024]);
$d = openssl_pkey_get_details($new);
var_dump($d['bits'], $d['type'], bin2hex($d['rsa']['e']));
$new = openssl_pkey_new(['private_key_type' => OPENSSL_KEYTYPE_EC, 'curve_name' => 'secp384r1']);
$d = openssl_pkey_get_details($new);
var_dump($d['bits'], $d['type'], $d['ec']['curve_name'], $d['ec']['curve_oid'], strlen($d['ec']['x']) <= 48);
var_dump(openssl_pkey_new(['private_key_type' => OPENSSL_KEYTYPE_EC]));
var_dump(openssl_pkey_new(['private_key_bits' => 128]));
$x = openssl_pkey_new(['private_key_type' => OPENSSL_KEYTYPE_EC, 'curve_name' => 'prime256v1']);
$y = openssl_pkey_new(['private_key_type' => OPENSSL_KEYTYPE_EC, 'curve_name' => 'prime256v1']);
$xp = openssl_pkey_get_public(openssl_pkey_get_details($x)['key']);
$yp = openssl_pkey_get_public(openssl_pkey_get_details($y)['key']);
var_dump(openssl_pkey_derive($yp, $x) === openssl_pkey_derive($xp, $y), strlen(openssl_pkey_derive($yp, $x)));
$ed = openssl_pkey_new(['private_key_type' => OPENSSL_KEYTYPE_ED25519]);
$d = openssl_pkey_get_details($ed);
var_dump($d['bits'], $d['type'], strlen($d['ed25519']['pub_key']));
var_dump(openssl_sign("m", $sig, $ed, 0), strlen($sig), openssl_verify("m", $sig, openssl_pkey_get_public($d['key']), 0));
$fixed = openssl_pkey_new(['ed25519' => ['priv_key' => str_repeat("\x01", 32)]]);
var_dump(openssl_sign("m", $sig, $fixed, 0), bin2hex($sig));
echo openssl_pkey_get_details($fixed)['key'];
try {
    new OpenSSLAsymmetricKey();
} catch (Error $e) {
    echo $e->getMessage(), "\n";
}
try {
    clone $k;
} catch (Error $e) {
    echo $e->getMessage(), "\n";
}
