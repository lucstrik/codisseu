//! Explicit, independently authored PHP/Blade and Rust live templates. These are templates,
//! not semantic suggestions, and never inspect or execute a project.
use std::{ops::Range, path::Path};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Context {
    Php,
    Blade,
    Rust,
    Python,
}
#[derive(Clone, Copy, Debug)]
pub struct Snippet {
    pub trigger: &'static str,
    pub description: &'static str,
    pub context: Context,
    pub body: &'static str,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Tabstop {
    pub number: u32,
    /// Unicode scalar offsets relative to the expanded text, never byte offsets.
    pub range: Range<usize>,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Expansion {
    /// Unicode scalar range in the original document, including the trigger.
    pub replace: Range<usize>,
    pub text: String,
    /// Ordered placeholders; the final cursor (number zero) is last.
    pub tabstops: Vec<Tabstop>,
}

// These small templates use public Laravel API names and original boilerplate.
// They do not contain extracted PhpStorm/VS Code assets or proprietary templates.
pub static CATALOG: &[Snippet] = &[
    Snippet {
        trigger: "def",
        description: "Python function",
        context: Context::Python,
        body: "def ${1:name}(${2:}):\n    $0",
    },
    Snippet {
        trigger: "asyncdef",
        description: "Python async function",
        context: Context::Python,
        body: "async def ${1:name}(${2:}):\n    $0",
    },
    Snippet {
        trigger: "class",
        description: "Python class",
        context: Context::Python,
        body: "class ${1:Name}:\n    def __init__(self, ${2:}):\n        $0",
    },
    Snippet {
        trigger: "main",
        description: "Python main guard",
        context: Context::Python,
        body: "if __name__ == \"__main__\":\n    $0",
    },
    Snippet {
        trigger: "for",
        description: "Python for loop",
        context: Context::Python,
        body: "for ${1:item} in ${2:items}:\n    $0",
    },
    Snippet {
        trigger: "while",
        description: "Python while loop",
        context: Context::Python,
        body: "while ${1:condition}:\n    $0",
    },
    Snippet {
        trigger: "if",
        description: "Python conditional",
        context: Context::Python,
        body: "if ${1:condition}:\n    $0",
    },
    Snippet {
        trigger: "try",
        description: "Python exception handling",
        context: Context::Python,
        body: "try:\n    ${1:pass}\nexcept ${2:Exception} as ${3:error}:\n    $0",
    },
    Snippet {
        trigger: "with",
        description: "Python context manager",
        context: Context::Python,
        body: "with ${1:open(\"file.txt\", encoding=\"utf-8\")} as ${2:stream}:\n    $0",
    },
    Snippet {
        trigger: "dataclass",
        description: "Python dataclass",
        context: Context::Python,
        body: "@dataclass\nclass ${1:Name}:\n    ${2:value}: ${3:str}\n    $0",
    },
    Snippet {
        trigger: "property",
        description: "Python property",
        context: Context::Python,
        body: "@property\ndef ${1:name}(self) -> ${2:str}:\n    $0",
    },
    Snippet {
        trigger: "test",
        description: "Python pytest test",
        context: Context::Python,
        body: "def test_${1:behavior}():\n    assert ${2:True}\n    $0",
    },
    Snippet {
        trigger: "unittest",
        description: "Python unittest case",
        context: Context::Python,
        body: "class Test${1:Behavior}(unittest.TestCase):\n    def test_${2:behavior}(self):\n        $0",
    },
    Snippet {
        trigger: "match",
        description: "Python structural match",
        context: Context::Python,
        body: "match ${1:value}:\n    case ${2:pattern}:\n        $0",
    },
    Snippet {
        trigger: "lambda",
        description: "Python lambda",
        context: Context::Python,
        body: "lambda ${1:value}: $0",
    },
    Snippet {
        trigger: "comp",
        description: "Python list comprehension",
        context: Context::Python,
        body: "[${1:item} for ${2:item} in ${3:items}]$0",
    },
    Snippet {
        trigger: "from",
        description: "Python named import",
        context: Context::Python,
        body: "from ${1:module} import ${2:name}$0",
    },
    Snippet {
        trigger: "fn",
        description: "Rust function",
        context: Context::Rust,
        body: "fn ${1:name}(${2:}) -> ${3:()} {\n    $0\n}",
    },
    Snippet {
        trigger: "pubfn",
        description: "Rust public function",
        context: Context::Rust,
        body: "pub fn ${1:name}(${2:}) -> ${3:()} {\n    $0\n}",
    },
    Snippet {
        trigger: "main",
        description: "Rust main function",
        context: Context::Rust,
        body: "fn main() {\n    $0\n}",
    },
    Snippet {
        trigger: "asyncfn",
        description: "Rust async function",
        context: Context::Rust,
        body: "async fn ${1:name}(${2:}) -> ${3:()} {\n    $0\n}",
    },
    Snippet {
        trigger: "test",
        description: "Rust unit test",
        context: Context::Rust,
        body: "#[test]\nfn ${1:it_works}() {\n    $0\n}",
    },
    Snippet {
        trigger: "modtest",
        description: "Rust tests module",
        context: Context::Rust,
        body: "#[cfg(test)]\nmod tests {\n    use super::*;\n\n    #[test]\n    fn ${1:it_works}() {\n        $0\n    }\n}",
    },
    Snippet {
        trigger: "struct",
        description: "Rust struct",
        context: Context::Rust,
        body: "struct ${1:Name} {\n    ${2:field}: ${3:String},\n    $0\n}",
    },
    Snippet {
        trigger: "enum",
        description: "Rust enum",
        context: Context::Rust,
        body: "enum ${1:Name} {\n    ${2:Variant},\n    $0\n}",
    },
    Snippet {
        trigger: "trait",
        description: "Rust trait",
        context: Context::Rust,
        body: "trait ${1:Name} {\n    fn ${2:method}(&self) -> ${3:()};\n    $0\n}",
    },
    Snippet {
        trigger: "impl",
        description: "Rust implementation",
        context: Context::Rust,
        body: "impl ${1:Name} {\n    $0\n}",
    },
    Snippet {
        trigger: "impltrait",
        description: "Rust trait implementation",
        context: Context::Rust,
        body: "impl ${1:Trait} for ${2:Type} {\n    $0\n}",
    },
    Snippet {
        trigger: "for",
        description: "Rust for loop",
        context: Context::Rust,
        body: "for ${1:item} in ${2:items} {\n    $0\n}",
    },
    Snippet {
        trigger: "while",
        description: "Rust while loop",
        context: Context::Rust,
        body: "while ${1:condition} {\n    $0\n}",
    },
    Snippet {
        trigger: "loop",
        description: "Rust loop",
        context: Context::Rust,
        body: "loop {\n    $0\n}",
    },
    Snippet {
        trigger: "match",
        description: "Rust match expression",
        context: Context::Rust,
        body: "match ${1:value} {\n    ${2:pattern} => ${3:result},\n    _ => {\n        $0\n    }\n}",
    },
    Snippet {
        trigger: "iflet",
        description: "Rust if let",
        context: Context::Rust,
        body: "if let ${1:Some(value)} = ${2:expression} {\n    $0\n}",
    },
    Snippet {
        trigger: "letelse",
        description: "Rust let else",
        context: Context::Rust,
        body: "let ${1:Some(value)} = ${2:expression} else {\n    ${3:return};\n};\n$0",
    },
    Snippet {
        trigger: "println",
        description: "Rust println macro",
        context: Context::Rust,
        body: "println!(\"${1:message}\");$0",
    },
    Snippet {
        trigger: "dbg",
        description: "Rust debug expression",
        context: Context::Rust,
        body: "dbg!(${1:value});$0",
    },
    Snippet {
        trigger: "derive",
        description: "Rust derive attribute",
        context: Context::Rust,
        body: "#[derive(${1:Debug, Clone})]\n$0",
    },
    Snippet {
        trigger: "result",
        description: "Rust Result-returning function",
        context: Context::Rust,
        body: "fn ${1:name}(${2:}) -> Result<${3:()}, ${4:Error}> {\n    $0\n    Ok(())\n}",
    },
    Snippet {
        trigger: "pubf",
        description: "Public PHP method",
        context: Context::Php,
        body: "public function ${1:method}(${2:}): ${3:void}\n{\n    $0\n}",
    },
    Snippet {
        trigger: "function",
        description: "PHP function",
        context: Context::Php,
        body: "function ${1:name}(${2:}): ${3:void}\n{\n    $0\n}",
    },
    Snippet {
        trigger: "class",
        description: "PHP class",
        context: Context::Php,
        body: "class ${1:Name}\n{\n    $0\n}",
    },
    Snippet {
        trigger: "interface",
        description: "PHP interface",
        context: Context::Php,
        body: "interface ${1:Name}\n{\n    $0\n}",
    },
    Snippet {
        trigger: "trait",
        description: "PHP trait",
        context: Context::Php,
        body: "trait ${1:Name}\n{\n    $0\n}",
    },
    Snippet {
        trigger: "if",
        description: "PHP conditional",
        context: Context::Php,
        body: "if (${1:$condition}) {\n    $0\n}",
    },
    Snippet {
        trigger: "elseif",
        description: "PHP alternative condition",
        context: Context::Php,
        body: "elseif (${1:$condition}) {\n    $0\n}",
    },
    Snippet {
        trigger: "for",
        description: "PHP for loop",
        context: Context::Php,
        body: "for (${1:$i = 0}; ${2:$i < $count}; ${3:$i++}) {\n    $0\n}",
    },
    Snippet {
        trigger: "foreach",
        description: "PHP foreach loop",
        context: Context::Php,
        body: "foreach (${1:$items} as ${2:$item}) {\n    $0\n}",
    },
    Snippet {
        trigger: "while",
        description: "PHP while loop",
        context: Context::Php,
        body: "while (${1:$condition}) {\n    $0\n}",
    },
    Snippet {
        trigger: "do",
        description: "PHP do-while loop",
        context: Context::Php,
        body: "do {\n    $0\n} while (${1:$condition});",
    },
    Snippet {
        trigger: "try",
        description: "PHP exception handling",
        context: Context::Php,
        body: "try {\n    ${1:}\n} catch (${2:\\Throwable} ${3:$error}) {\n    $0\n}",
    },
    Snippet {
        trigger: "throw",
        description: "PHP throw exception",
        context: Context::Php,
        body: "throw new ${1:\\RuntimeException}('${2:message}');$0",
    },
    Snippet {
        trigger: "return",
        description: "PHP return value",
        context: Context::Php,
        body: "return ${1:$value};$0",
    },
    Snippet {
        trigger: "match",
        description: "PHP match expression (PHP 8+)",
        context: Context::Php,
        body: "match (${1:$value}) {\n    ${2:condition} => ${3:result},\n    default => ${4:null},\n};$0",
    },
    Snippet {
        trigger: "construct",
        description: "PHP constructor",
        context: Context::Php,
        body: "public function __construct(${1:})\n{\n    $0\n}",
    },
    Snippet {
        trigger: "pubsf",
        description: "Public static PHP method",
        context: Context::Php,
        body: "public static function ${1:method}(${2:}): ${3:void}\n{\n    $0\n}",
    },
    Snippet {
        trigger: "prof",
        description: "Protected PHP method",
        context: Context::Php,
        body: "protected function ${1:method}(${2:}): ${3:void}\n{\n    $0\n}",
    },
    Snippet {
        trigger: "prif",
        description: "Private PHP method",
        context: Context::Php,
        body: "private function ${1:method}(${2:}): ${3:void}\n{\n    $0\n}",
    },
    Snippet {
        trigger: "fore",
        description: "PHP foreach loop",
        context: Context::Php,
        body: "foreach (${1:$items} as ${2:$item}) {\n    $0\n}",
    },
    Snippet {
        trigger: "eco",
        description: "PHP echo",
        context: Context::Php,
        body: "echo ${1:$value};$0",
    },
    Snippet {
        trigger: "lrequest",
        description: "Laravel form request",
        context: Context::Php,
        body: "namespace App\\Http\\Requests;\n\nuse Illuminate\\Foundation\\Http\\FormRequest;\n\nclass ${1:ExampleRequest} extends FormRequest\n{\n    public function authorize(): bool\n    {\n        return ${2:true};\n    }\n\n    public function rules(): array\n    {\n        return [\n            '${3:name}' => ['${4:required}', '${5:string}'],\n        ];\n    }\n    $0\n}",
    },
    Snippet {
        trigger: "lresource",
        description: "Laravel JSON resource",
        context: Context::Php,
        body: "namespace App\\Http\\Resources;\n\nuse Illuminate\\Http\\Request;\nuse Illuminate\\Http\\Resources\\Json\\JsonResource;\n\nclass ${1:ExampleResource} extends JsonResource\n{\n    public function toArray(Request $request): array\n    {\n        return [\n            '${2:id}' => $this->${3:id},\n            $0\n        ];\n    }\n}",
    },
    Snippet {
        trigger: "lcommand",
        description: "Laravel Artisan command",
        context: Context::Php,
        body: "namespace App\\Console\\Commands;\n\nuse Illuminate\\Console\\Command;\n\nclass ${1:ExampleCommand} extends Command\n{\n    protected $signature = '${2:example:run}';\n    protected $description = '${3:Description}';\n\n    public function handle(): int\n    {\n        $0\n        return self::SUCCESS;\n    }\n}",
    },
    Snippet {
        trigger: "ljob",
        description: "Laravel queued job",
        context: Context::Php,
        body: "namespace App\\Jobs;\n\nuse Illuminate\\Bus\\Queueable;\nuse Illuminate\\Contracts\\Queue\\ShouldQueue;\nuse Illuminate\\Foundation\\Bus\\Dispatchable;\nuse Illuminate\\Queue\\InteractsWithQueue;\nuse Illuminate\\Queue\\SerializesModels;\n\nclass ${1:ExampleJob} implements ShouldQueue\n{\n    use Dispatchable, InteractsWithQueue, Queueable, SerializesModels;\n\n    public function handle(): void\n    {\n        $0\n    }\n}",
    },
    Snippet {
        trigger: "lpolicy",
        description: "Laravel policy method",
        context: Context::Php,
        body: "public function ${1:update}(\\App\\Models\\User $user, ${2:Example} $model): bool\n{\n    return ${3:$user->id === $model->user_id};\n}$0",
    },
    Snippet {
        trigger: "lhasone",
        description: "Eloquent hasOne relationship",
        context: Context::Php,
        body: "public function ${1:profile}(): \\Illuminate\\Database\\Eloquent\\Relations\\HasOne\n{\n    return $this->hasOne(${2:Profile}::class);\n}$0",
    },
    Snippet {
        trigger: "lbelongsmany",
        description: "Eloquent belongsToMany relationship",
        context: Context::Php,
        body: "public function ${1:roles}(): \\Illuminate\\Database\\Eloquent\\Relations\\BelongsToMany\n{\n    return $this->belongsToMany(${2:Role}::class);\n}$0",
    },
    Snippet {
        trigger: "lmorphmany",
        description: "Eloquent morphMany relationship",
        context: Context::Php,
        body: "public function ${1:comments}(): \\Illuminate\\Database\\Eloquent\\Relations\\MorphMany\n{\n    return $this->morphMany(${2:Comment}::class, '${3:commentable}');\n}$0",
    },
    Snippet {
        trigger: "ltest",
        description: "Laravel PHPUnit feature test",
        context: Context::Php,
        body: "public function test_${1:example}(): void\n{\n    $response = $this->${2:getJson}('${3:/api/example}');\n    $response->assert${4:Ok}();\n    $0\n}",
    },
    Snippet {
        trigger: "@unless",
        description: "Blade unless block",
        context: Context::Blade,
        body: "@unless (${1:$condition})\n    $0\n@endunless",
    },
    Snippet {
        trigger: "@isset",
        description: "Blade isset block",
        context: Context::Blade,
        body: "@isset (${1:$value})\n    $0\n@endisset",
    },
    Snippet {
        trigger: "@empty",
        description: "Blade empty block",
        context: Context::Blade,
        body: "@empty (${1:$items})\n    $0\n@endempty",
    },
    Snippet {
        trigger: "@switch",
        description: "Blade switch block",
        context: Context::Blade,
        body: "@switch (${1:$value})\n    @case (${2:1})\n        $0\n        @break\n    @default\n        ${3:}\n@endswitch",
    },
    Snippet {
        trigger: "@push",
        description: "Blade push stack",
        context: Context::Blade,
        body: "@push('${1:scripts}')\n    $0\n@endpush",
    },
    Snippet {
        trigger: "@stack",
        description: "Blade render stack",
        context: Context::Blade,
        body: "@stack('${1:scripts}')$0",
    },
    Snippet {
        trigger: "@props",
        description: "Blade component props",
        context: Context::Blade,
        body: "@props(['${1:title}' => ${2:null}])$0",
    },
    Snippet {
        trigger: "@slot",
        description: "Blade named slot",
        context: Context::Blade,
        body: "<x-slot:${1:header}>\n    $0\n</x-slot>",
    },
    Snippet {
        trigger: "@once",
        description: "Blade once block",
        context: Context::Blade,
        body: "@once\n    $0\n@endonce",
    },
    Snippet {
        trigger: "@php",
        description: "Blade PHP block",
        context: Context::Blade,
        body: "@php\n    $0\n@endphp",
    },
    Snippet {
        trigger: "@vite",
        description: "Blade Vite entry points (Laravel 9.19+)",
        context: Context::Blade,
        body: "@vite(['${1:resources/css/app.css}', '${2:resources/js/app.js}'])$0",
    },
    Snippet {
        trigger: "lcontroller",
        description: "Laravel controller class",
        context: Context::Php,
        body: "namespace App\\Http\\Controllers;\n\nuse Illuminate\\Http\\Request;\n\nclass ${1:ExampleController} extends Controller\n{\n    public function ${2:index}(Request $request)\n    {\n        $0\n    }\n}",
    },
    Snippet {
        trigger: "lmodel",
        description: "Eloquent model class",
        context: Context::Php,
        body: "namespace App\\Models;\n\nuse Illuminate\\Database\\Eloquent\\Model;\n\nclass ${1:Example} extends Model\n{\n    protected $fillable = [${2:'name'}];\n\n    $0\n}",
    },
    Snippet {
        trigger: "lroute",
        description: "Route with controller action",
        context: Context::Php,
        body: "Route::${1:get}('${2:/path}', [${3:ExampleController}::class, '${4:index}'])->name('${5:example.index}');$0",
    },
    Snippet {
        trigger: "lview",
        description: "Return a Blade view",
        context: Context::Php,
        body: "return view('${1:example.index}', [${2:}]);$0",
    },
    Snippet {
        trigger: "lvalidate",
        description: "Validate a request",
        context: Context::Php,
        body: "$validated = $request->validate([\n    '${1:name}' => ['${2:required}', '${3:string}'],\n]);$0",
    },
    Snippet {
        trigger: "lmigration",
        description: "Anonymous migration (Laravel 8.37+)",
        context: Context::Php,
        body: "use Illuminate\\Database\\Migrations\\Migration;\nuse Illuminate\\Database\\Schema\\Blueprint;\nuse Illuminate\\Support\\Facades\\Schema;\n\nreturn new class extends Migration\n{\n    public function up(): void\n    {\n        Schema::create('${1:examples}', function (Blueprint $table) {\n            $table->id();\n            $0\n            $table->timestamps();\n        });\n    }\n\n    public function down(): void\n    {\n        Schema::dropIfExists('${2:examples}');\n    }\n};",
    },
    Snippet {
        trigger: "lbelongs",
        description: "Eloquent belongsTo relationship",
        context: Context::Php,
        body: "public function ${1:owner}(): \\Illuminate\\Database\\Eloquent\\Relations\\BelongsTo\n{\n    return $this->belongsTo(${2:User}::class);\n}$0",
    },
    Snippet {
        trigger: "lhasmany",
        description: "Eloquent hasMany relationship",
        context: Context::Php,
        body: "public function ${1:items}(): \\Illuminate\\Database\\Eloquent\\Relations\\HasMany\n{\n    return $this->hasMany(${2:Item}::class);\n}$0",
    },
    Snippet {
        trigger: "lscope",
        description: "Classic Eloquent query scope",
        context: Context::Php,
        body: "public function scope${1:Active}(\\Illuminate\\Database\\Eloquent\\Builder $query): \\Illuminate\\Database\\Eloquent\\Builder\n{\n    return $query->where('${2:active}', ${3:true});\n}$0",
    },
    Snippet {
        trigger: "ltransaction",
        description: "Database transaction callback",
        context: Context::Php,
        body: "\\Illuminate\\Support\\Facades\\DB::transaction(function () {\n    $0\n});",
    },
    Snippet {
        trigger: "@if",
        description: "Blade if block",
        context: Context::Blade,
        body: "@if (${1:$condition})\n    $0\n@endif",
    },
    Snippet {
        trigger: "@foreach",
        description: "Blade foreach block",
        context: Context::Blade,
        body: "@foreach (${1:$items} as ${2:$item})\n    $0\n@endforeach",
    },
    Snippet {
        trigger: "@forelse",
        description: "Blade forelse with empty state",
        context: Context::Blade,
        body: "@forelse (${1:$items} as ${2:$item})\n    $0\n@empty\n    ${3:No items}\n@endforelse",
    },
    Snippet {
        trigger: "@section",
        description: "Blade section block",
        context: Context::Blade,
        body: "@section('${1:content}')\n    $0\n@endsection",
    },
    Snippet {
        trigger: "@extends",
        description: "Blade layout inheritance",
        context: Context::Blade,
        body: "@extends('${1:layouts.app}')\n\n@section('${2:content}')\n    $0\n@endsection",
    },
    Snippet {
        trigger: "@include",
        description: "Blade include",
        context: Context::Blade,
        body: "@include('${1:partials.example}', [${2:}])$0",
    },
    Snippet {
        trigger: "@auth",
        description: "Blade authenticated block",
        context: Context::Blade,
        body: "@auth\n    $0\n@endauth",
    },
    Snippet {
        trigger: "@can",
        description: "Blade authorization block",
        context: Context::Blade,
        body: "@can('${1:update}', ${2:$model})\n    $0\n@endcan",
    },
    Snippet {
        trigger: "@error",
        description: "Blade validation message",
        context: Context::Blade,
        body: "@error('${1:field}')\n    <span>{{ $message }}</span>\n@enderror$0",
    },
    Snippet {
        trigger: "@csrf",
        description: "Blade CSRF field",
        context: Context::Blade,
        body: "@csrf$0",
    },
    Snippet {
        trigger: "@method",
        description: "Blade HTTP method field",
        context: Context::Blade,
        body: "@method('${1:PUT}')$0",
    },
];

pub fn context(path: &Path) -> Option<Context> {
    let name = path.file_name()?.to_str()?;
    if name.ends_with(".blade.php") {
        Some(Context::Blade)
    } else if path.extension().is_some_and(|v| v == "php") {
        Some(Context::Php)
    } else if path.extension().is_some_and(|v| v == "rs") {
        Some(Context::Rust)
    } else if matches!(
        path.extension().and_then(|e| e.to_str()),
        Some("py" | "pyi" | "pyw")
    ) {
        Some(Context::Python)
    } else {
        None
    }
}

pub fn candidates<'a>(path: &Path, prefix: &'a str) -> impl Iterator<Item = &'static Snippet> + 'a {
    let context = context(path);
    CATALOG
        .iter()
        .filter(move |s| Some(s.context) == context && s.trigger.starts_with(prefix))
}

/// Expand an exact, explicitly requested trigger ending at `caret`.
/// This operation performs no write; the editor must apply a versioned edit.
/// Tabs remain actual tabs, CRLF remains CRLF, and indentation is inherited.
pub fn expand(path: &Path, text: &str, caret: usize) -> Option<Expansion> {
    let byte = text
        .char_indices()
        .nth(caret)
        .map(|(b, _)| b)
        .or_else(|| (caret == text.chars().count()).then_some(text.len()))?;
    let prefix = &text[..byte];
    let snippet = CATALOG
        .iter()
        .filter(|s| Some(s.context) == context(path) && prefix.ends_with(s.trigger))
        .max_by_key(|s| s.trigger.len())?;
    let start = byte.checked_sub(snippet.trigger.len())?;
    if text[..start]
        .chars()
        .next_back()
        .is_some_and(|c| c.is_alphanumeric() || matches!(c, '_' | '$' | '@'))
    {
        return None;
    }
    let line_start = text[..start].rfind('\n').map_or(0, |i| i + 1);
    let indent: String = text[line_start..start]
        .chars()
        .take_while(|c| matches!(c, ' ' | '\t'))
        .collect();
    let line_break = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let body = snippet.body.replace('\n', &format!("{line_break}{indent}"));
    let (expanded, tabstops) = render(&body)?;
    Some(Expansion {
        replace: text[..start].chars().count()..caret,
        text: expanded,
        tabstops,
    })
}

fn render(body: &str) -> Option<(String, Vec<Tabstop>)> {
    let mut out = String::new();
    let mut stops = Vec::new();
    let mut rest = body;
    while !rest.is_empty() {
        if let Some(next) = rest.strip_prefix("$0") {
            let n = out.chars().count();
            stops.push(Tabstop {
                number: 0,
                range: n..n,
            });
            rest = next;
        } else if let Some(next) = rest.strip_prefix("${") {
            let colon = next.find(':')?;
            let number: u32 = next[..colon].parse().ok()?;
            if number == 0 {
                return None;
            }
            let tail = &next[colon + 1..];
            let end = tail.find('}')?;
            let default = &tail[..end];
            let start = out.chars().count();
            out.push_str(default);
            stops.push(Tabstop {
                number,
                range: start..out.chars().count(),
            });
            rest = &tail[end + 1..];
        } else {
            let ch = rest.chars().next()?;
            out.push(ch);
            rest = &rest[ch.len_utf8()..];
        }
    }
    stops.sort_by_key(|s| if s.number == 0 { u32::MAX } else { s.number });
    if stops.windows(2).any(|s| s[0].number == s[1].number) {
        return None;
    }
    Some((out, stops))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn catalog_is_context_scoped_and_every_template_expands() {
        for snippet in CATALOG {
            let path = Path::new(match snippet.context {
                Context::Blade => "hello.blade.php",
                Context::Php => "Controller.php",
                Context::Rust => "main.rs",
                Context::Python => "main.py",
            });
            let exp = expand(path, snippet.trigger, snippet.trigger.chars().count()).unwrap();
            assert_eq!(exp.replace, 0..snippet.trigger.chars().count());
            assert!(!exp.text.contains("${1:"));
            assert_eq!(exp.tabstops.last().unwrap().number, 0);
            assert!(
                exp.tabstops
                    .iter()
                    .all(|s| s.range.end <= exp.text.chars().count())
            );
        }
        assert!(candidates(Path::new("x.ts"), "l").next().is_none());
        assert!(expand(Path::new("x.php"), "@foreach", 8).is_none());
    }
    #[test]
    fn unicode_crlf_indentation_and_scalar_ranges_are_preserved() {
        let text = "<?php // 🌧\r\n\t  lvalidate";
        let exp = expand(Path::new("test.php"), text, text.chars().count()).unwrap();
        assert_eq!(exp.replace, 15..24);
        assert!(exp.text.contains("\r\n\t      'name'"));
        let first = &exp.tabstops[0];
        assert_eq!(
            exp.text
                .chars()
                .skip(first.range.start)
                .take(first.range.len())
                .collect::<String>(),
            "name"
        );
    }
    #[test]
    fn expansion_requires_an_exact_whole_trigger_and_valid_caret() {
        let path = Path::new("routes.php");
        for s in ["mylroute", "$lroute", "lrouteX", "@lroute"] {
            assert!(expand(path, s, s.chars().count()).is_none());
        }
        assert!(expand(path, "lroute", 500).is_none());
        assert!(expand(path, "lroute", 3).is_none());
        assert_eq!(candidates(path, "lrou").next().unwrap().trigger, "lroute");
    }
    #[test]
    fn placeholders_and_final_cursor_have_stable_order() {
        let exp = expand(Path::new("a.blade.php"), "@forelse", 8).unwrap();
        assert_eq!(
            exp.tabstops.iter().map(|s| s.number).collect::<Vec<_>>(),
            vec![1, 2, 3, 0]
        );
        assert!(exp.text.contains("$items as $item"));
        assert!(render("${1:x}${1:y}$0").is_none());
    }
    #[test]
    fn rust_templates_keep_longest_trigger_unicode_crlf_and_language_scope() {
        let source = "// 😀\r\n\tpubfn";
        let exp = expand(Path::new("main.rs"), source, source.chars().count()).unwrap();
        assert!(exp.text.starts_with("pub fn name("));
        assert!(exp.text.contains("\r\n\t    "));
        assert_eq!(
            exp.replace,
            source.chars().count() - 5..source.chars().count()
        );
        assert_eq!(candidates(Path::new("main.rs"), "impl").count(), 2);
        assert!(candidates(Path::new("main.php"), "impl").next().is_none());
        assert!(expand(Path::new("main.rs"), "lroute", 6).is_none());
        assert!(expand(Path::new("main.rs"), "myfn", 4).is_none());
        let exp = expand(Path::new("main.rs"), "asyncfn", 7).unwrap();
        assert!(exp.text.starts_with("async fn name("));
        assert_eq!(
            exp.tabstops.iter().map(|t| t.number).collect::<Vec<_>>(),
            [1, 2, 3, 0]
        );
    }
}
