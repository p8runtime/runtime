<?php
// A template: $props is the data the route passed to renderToResponse.
$name = htmlspecialchars($props['name'] ?? 'nobody');
?>
<article class="card">
  <h2><?= $name ?></h2>
</article>
