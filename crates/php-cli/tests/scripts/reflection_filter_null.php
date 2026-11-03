<?php
class A { function a() {} static function s() {} private function p() {} }
var_dump(count((new ReflectionClass('A'))->getMethods()), count((new ReflectionClass('A'))->getMethods(null)), count((new ReflectionClass('A'))->getMethods(ReflectionMethod::IS_STATIC)));
var_dump(count((new ReflectionClass('A'))->getProperties(null)), count((new ReflectionClass('A'))->getConstants(null)));
