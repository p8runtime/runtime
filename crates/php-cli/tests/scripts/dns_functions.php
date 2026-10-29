<?php
// Host names that need no network: addresses, failures.
var_dump(gethostbyaddr('not an address'));
var_dump(gethostbyname('10.1.2.3'), gethostbynamel('10.1.2.3'));
var_dump(gethostbyname('no-such-host.invalid'), gethostbynamel('no-such-host.invalid'));
var_dump(gethostbyname(str_repeat('a', 300)) === str_repeat('a', 300));
var_dump(gethostbynamel(str_repeat('a', 300)));
var_dump(is_string(gethostname()));
