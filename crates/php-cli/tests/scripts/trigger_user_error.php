<?php
// trigger_error(E_USER_ERROR): PHP 8.4's deprecation first, then the error,
// both to a user handler (which may throw, or decline: fatal); @ only
// changes error_reporting() inside it.
set_error_handler(function ($no, $str) { echo "handler($no, ", error_reporting(), "): $str\n"; if ($no === E_USER_DEPRECATED || $no === E_DEPRECATED) { throw new Exception("from handler"); } return true; });
try { @trigger_error('e1', E_USER_ERROR); } catch (Exception $e) { echo "caught ", $e->getMessage(), "\n"; }
ob_start(function ($buf) { try { @trigger_error('in ob', E_USER_ERROR); } catch (Exception $e) { return "ob-caught\n"; } return $buf; });
echo "body\n";
// (8.6: the handler's own output is deprecated; that deprecation reaches the
// error handler above, which throws, out of ob_end_flush() once it is done.)
try { ob_end_flush(); } catch (Exception $e) { echo "ob_end_flush: ", $e->getMessage(), "\n"; }
set_error_handler(function ($no, $str) { echo "takes $no: $str\n"; return true; });
var_dump(trigger_error('handled', E_USER_ERROR));
set_error_handler(function ($no, $str) { echo "declines $no\n"; return false; });
trigger_error('declined', E_USER_ERROR);
echo "not reached\n";
