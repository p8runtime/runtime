<?php
interface I { function i(); }
trait T { function t() {} private function tp() {} }
class P implements I { function i() {} function p() {} protected function prot() {} private function priv() {} static function st() {} }
class C extends P { use T; function c() {} function p() {} private function own() {} function list() { return get_class_methods($this); } }
print_r(get_class_methods('C')); print_r((new C)->list()); print_r(get_class_methods(new P));
