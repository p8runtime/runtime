<?php
function is_string_keyed($v, $k) {
  return is_string($v);
}
echo array_any(['red', 'blue'], 'is_string_keyed') ? 'yes' : 'no';
