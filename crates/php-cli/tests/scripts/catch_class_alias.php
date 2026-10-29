<?php
// catch, instanceof, is_a() and is_subclass_of() through a class_alias()'d name
// (WordPress's tests catch PHPUnit_Framework_ExpectationFailedException, an
// alias of PHPUnit\Framework\ExpectationFailedException).
class Real extends Exception {}
class_alias("Real", "Old_Name");
interface I {} class_alias("I", "IAlias"); class W extends Exception implements I {}
try { throw new Real("x"); } catch (Old_Name $e) { echo "caught by alias\n"; }
try { throw new W("y"); } catch (IAlias $e) { echo "caught by interface alias\n"; }
var_dump(new Real instanceof Old_Name, is_a(new W, "IAlias"));
try { throw new Real; } catch (Undefined_Class $e) { echo "no\n"; } catch (Exception $e) { echo "falls through\n"; }
var_dump(is_subclass_of(new Real, "Old_Name"), is_subclass_of("Real", "Exception"), new Real instanceof IAlias);
