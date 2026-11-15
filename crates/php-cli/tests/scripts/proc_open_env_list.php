<?php
// proc_open()'s $env_vars as PHP makes the child's environment of it
// (php_array_to_envp): a string key names the variable; an entry under an
// integer key is the whole "NAME=value" (Symfony Process passes its
// environment so: Composer's and Laravel's processes found no PATH, #563);
// an empty value is left out.
function child(array|string $command, array $env): string
{
    $p = proc_open($command, [1 => ['pipe', 'w'], 2 => ['pipe', 'w']], $pipes, null, $env);
    $out = stream_get_contents($pipes[1]) . stream_get_contents($pipes[2]);
    fclose($pipes[1]);
    fclose($pipes[2]);
    proc_close($p);
    return $out;
}

echo child('echo "A=$A B=$B C=$C E=${E-unset} F=${F-unset} G=$G"', [
    'A=1',
    'B=x=y',
    'C' => 'named',
    'E' => '',
    'F=',
    5 => 'G=g',
    'no-equals',
]);
// A list naming PATH: the child's PATH, and the one its program is found on.
echo child(['sh', '-c', 'echo "PATH=$PATH"; echo "${P8_ENV_LIST-unset}"'], ['PATH=/bin:/usr/bin', 'P8_ENV_LIST=listed']);
// Keys as PHP turns them into array keys: "7" is the integer 7.
echo child('echo "${X-unset} ${Y-unset}"', ['7' => 'X=seven', 'Y' => 'why']);
