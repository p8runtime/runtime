<?php
// A builtin attribute on the wrong kind of declaration is a compile error.

#[Attribute]
function a1() {}
