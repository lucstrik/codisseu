# Rename web audit fixtures

Own synthetic sources for cross-file JavaScript/TypeScript method rename.
Rename only Weather.rainfall to forecast, including main and closed callers.
Other.rainfall and ordinary strings/comments retain their exact bytes.
The JavaScript project deliberately has neither checkJs nor JSDoc. Removing
its jsconfig.json yields the corresponding inferred-project scenario.
No dependencies, installs, build hooks or network access are required.
