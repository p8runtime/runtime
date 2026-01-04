<?php
print_r(sscanf("42 0x2a 052 -7 Z", "%u %x %o %d %c"));
print_r(sscanf("abc def", "%3c %s"));
print_r(sscanf("ID:99", "ID:%u"));
print_r(sscanf("skip:123", "skip:%*d"));
print_r(sscanf("A   5", "A %d"));
