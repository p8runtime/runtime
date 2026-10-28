<?php
$values = [0, 1, -1, 42, -42, 255, PHP_INT_MAX, PHP_INT_MIN, 3.14159, -2.5, 0.5, 1.005, 1e20, -0.0, 0.000123456, 123456789.987, "12abc", "x", true, null, INF, -INF, NAN];
$formats = ['%d', '%5d', '%-5d|', '%05d', '%-05d|', '%+d', '%+05d', '%u', '%+05u', '%x', '%X', '%o', '%b', '%c', '%e', '%.2e', '%E', '%f', '%.0f', '%.2f', '%10.3f', '%-10.2f|', '%+.1f', '%F', '%g', '%G', '%.3g', '%h', '%H', '%s', '%5s', '%-5s|', '%.2s', "%'*8s", "%'#10.3f", '%08.3f', '%1$x %1$X %1$o %1$b', '%5.1e'];
foreach ($formats as $f) {
    $row = [];
    foreach ($values as $v) $row[] = sprintf($f, $v);
    echo $f, ' => ', implode(' | ', $row), "\n";
}
echo sprintf('%2$s %1$s %2$s %1$05d', 'a', 'b'), "\n";
echo sprintf('%1$s %s %s', 'x', 'y'), "\n";
echo sprintf('[%*d] [%-*s] [%.*f] [%*.*f]', 6, 42, 4, 'ab', 2, 3.14159, 8, 3, 2.71828), "\n";
echo sprintf('[%1$*2$d] [%3$.*2$f]', 7, 3, 1.5), "\n";
function t(callable $f) { try { echo $f(), "\n"; } catch (Throwable $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; } }
t(fn() => sprintf('%.-1g', 0.1));
t(fn() => sprintf('%.*g|%.*h|%.*G', -1, 1/3, -1, 1e100, -1, 0.1));
t(fn() => sprintf('%5%|%-5%|%%|%ld', 1, 2, 3));
t(fn() => sprintf('%.60f', 1.5));
echo vsprintf('%2$s-%1$s', ['a', 'b']), " ", printf("%s=%d\n", 'p', 7), "\n";
class S { function __toString(): string { return 'str!'; } }
echo sprintf('%s %5s %.2s', new S, new S, new S), "\n";
echo vsprintf('%s', [new S]), "\n";
echo sprintf('%s', [1]), "\n";
foreach (['%', '%0', '%y', '%0$s', '%*s', '%.*s', '%.-1s', '%2$s', "%'", '%1$*0$d'] as $bad) {
    try { var_dump(sprintf($bad, 'a')); } catch (Throwable $e) { echo $bad, ' ', get_class($e), ': ', $e->getMessage(), "\n"; }
}
try { vsprintf('%s %s', ['a']); } catch (Throwable $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; }
try { sprintf('%s %s %3$s', 'a'); } catch (Throwable $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; }
try { fprintf(STDOUT, '%s %s', 'a'); } catch (Throwable $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; }
try { sprintf('%*d', 'x', 1); } catch (Throwable $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; }
try { sprintf('%s', new stdClass); } catch (Throwable $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; }
