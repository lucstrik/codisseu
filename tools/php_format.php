<?php
// Installation-owned helper. Never require/evaluate the user's source/config.
declare(strict_types=1);
try {
    if ($argc !== 4) { throw new RuntimeException('Invalid formatter invocation'); }
    require 'phar://' . $argv[1] . '/vendor/autoload.php';
    $source = file_get_contents($argv[2]);
    if ($source === false || strlen($source) > 256 * 1024) { throw new RuntimeException('PHP input too large'); }
    $signature = static function (string $code): array {
        $result = [];
        foreach (token_get_all($code, TOKEN_PARSE) as $token) {
            if (is_array($token)) {
                if ($token[0] === T_WHITESPACE) { continue; }
                $result[] = [$token[0], $token[0] === T_OPEN_TAG ? rtrim($token[1]) : $token[1]];
            } else { $result[] = $token; }
        }
        return $result;
    };
    $before = $signature($source);
    $eol = str_contains($source, "\r\n") ? "\r\n" : "\n";
    $rules = array_fill_keys([
        'array_indentation', 'binary_operator_spaces', 'blank_line_after_opening_tag',
        'braces_position', 'compact_nullable_type_declaration', 'function_declaration',
        'indentation_type', 'method_argument_space', 'no_trailing_whitespace',
        'no_whitespace_before_comma_in_array', 'object_operator_without_whitespace',
        'single_space_around_construct', 'spaces_inside_parentheses',
        'statement_indentation', 'trim_array_spaces', 'type_declaration_spaces',
        'unary_operator_spaces', 'whitespace_after_comma_in_array',
    ], true);
    $rules['concat_space'] = ['spacing' => 'one'];
    $factory = new PhpCsFixer\FixerFactory();
    $factory->registerBuiltInFixers();
    $factory->setWhitespacesConfig(new PhpCsFixer\WhitespacesFixerConfig('    ', $eol));
    $factory->useRuleSet(new PhpCsFixer\RuleSet\RuleSet($rules));
    $tokens = PhpCsFixer\Tokenizer\Tokens::fromCode($source);
    $file = new SplFileInfo($argv[2]);
    foreach ($factory->getFixers() as $fixer) {
        if ($fixer->isRisky()) { throw new RuntimeException('Risky formatting rule refused'); }
        if ($fixer->supports($file) && $fixer->isCandidate($tokens)) { $fixer->fix($file, $tokens); }
    }
    $result = $tokens->generateCode();
    if (!str_ends_with($source, "\n")) { $result = rtrim($result, "\r\n"); }
    elseif (!str_ends_with($result, "\n")) { $result .= $eol; }
    if (strlen($result) > 1024 * 1024 || $signature($result) !== $before) {
        throw new RuntimeException('Formatting would change PHP tokens or literals');
    }
    if (file_put_contents($argv[3], $result) === false) { throw new RuntimeException('Cannot write formatter result'); }
} catch (Throwable $error) {
    // No source excerpts or project details in diagnostics.
    fwrite(STDERR, 'PHP formatting failed: ' . get_class($error) . "\n");
    exit(1);
}
