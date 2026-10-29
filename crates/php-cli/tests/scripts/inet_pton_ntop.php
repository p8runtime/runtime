<?php
// inet_pton()/inet_ntop() as glibc writes addresses: the longest run of
// zero groups (the first, on a tie, never a single one) as ::, mapped and
// compatible IPv4 as a dotted quad; false for what is not an address.
// (WordPress anonymises IPv6 with inet_pton, a & mask, inet_ntop.)
foreach (['1.2.3.4', '0.0.0.0', '255.255.255.255', '::', '::1', '1::', '1:0:0:2:0:0:3:4', '1:0:0:0:2:0:0:0', '1:0:2:0:3:0:4:0',
          '::ffff:1.2.3.4', '::1.2.3.4', '::ffff:0:1.2.3.4', 'FE80::0202:B3FF:FE1E:8329', '2001:db8::ff00:42:8329', '0:0:1::'] as $ip) {
    $bin = inet_pton($ip);
    echo str_pad($ip, 28), bin2hex($bin), '  ', inet_ntop($bin), "\n";
}
var_dump(inet_pton('nope'), inet_pton(''), inet_pton('1.2.3'), inet_pton('01.2.3.4'), inet_pton('1::2::3'), inet_ntop('abc'));
$mask = inet_pton('ffff:ffff:ffff:ffff::');
var_dump(inet_ntop(inet_pton('2a03:2880:2110:df07:face:b00c::1') & $mask));
