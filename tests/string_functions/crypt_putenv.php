<?php
foreach (['rasmuslerdorf' => ['rl', '_J9..rasm', '$1$rasmusle$', '$2y$07$usesomesillystringforsalt$', '$5$rounds=5000$usesomesillystringforsalt$', '$6$rounds=5000$usesomesillystringforsalt$', '*0', 'x']] as $pw => $salts) foreach ($salts as $s) var_dump(crypt($pw, $s));
var_dump(putenv("PHPRS_T=1"), getenv("PHPRS_T"), putenv("PHPRS_T="), getenv("PHPRS_T"), putenv("PHPRS_T"), getenv("PHPRS_T"));
putenv("PHPRS_C=child"); echo shell_exec('echo $PHPRS_C');
var_dump(is_array(getenv()), function_exists('money_format'));
