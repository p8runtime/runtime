<?php
// A child process starts in the working directory the script moved to;
// proc_open's $cwd is itself relative to it (#459). Prints no path of
// this machine.
$base = sys_get_temp_dir() . '/p8-cwd-child-' . getmypid();
mkdir($base);
mkdir("$base/sub");
chdir($base);
$real = getcwd();
file_put_contents('here.txt', "a file here\n");

$pwd = fn ($out) => trim($out) === $real ? 'base' : (trim($out) === "$real/sub" ? 'sub' : 'elsewhere');

// (Backticks are shell_exec(); left out while p8 lacks 8.6's deprecation
// of them, #467. popen() is #466.)
echo $pwd(shell_exec('pwd -P')), "\n";
exec('pwd -P', $lines, $status);
echo $pwd($lines[0]), " $status\n";
ob_start();
system('pwd -P');
echo $pwd(ob_get_clean()), "\n";
ob_start();
passthru('cat here.txt');
echo ob_get_clean();

$spec = [1 => ['pipe', 'w']];
$proc = proc_open('pwd -P', $spec, $pipes);
echo $pwd(stream_get_contents($pipes[1])), "\n";
fclose($pipes[1]);
proc_close($proc);
$proc = proc_open('pwd -P', $spec, $pipes, 'sub');
echo $pwd(stream_get_contents($pipes[1])), "\n";
fclose($pipes[1]);
proc_close($proc);
$proc = proc_open(['cat', 'here.txt'], $spec, $pipes);
echo stream_get_contents($pipes[1]);
fclose($pipes[1]);
proc_close($proc);

chdir('sub');
echo $pwd(shell_exec('pwd -P')), "\n";
echo shell_exec('cat ../here.txt');
chdir('..');

unlink('here.txt');
rmdir('sub');
chdir(sys_get_temp_dir());
var_dump(rmdir($base));
