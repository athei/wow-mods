-- Deterministic string-library records for comparison across builds.
-- Run with Lua 5.0 and compare the returned string byte for byte.
local records = {}
local function pack(...) return arg end
local function encode(value)
    if type(value) == "string" then
        return "s:" .. string.gsub(value, ".", function(c) return string.format("%02x", string.byte(c)) end)
    end
    return type(value) .. ":" .. tostring(value)
end
local function record(values)
    local fields = {tostring(values.n)}
    for i = 1, values.n do table.insert(fields, encode(values[i])) end
    table.insert(records, table.concat(fields, "|"))
end
local subjects = {"", "abcd", "abcdabcd", "Your Fireball hits Target for 123 damage.",
    "nothing matching here", "abcd\000tail", "\000abcd", string.rep("x", 256) .. "damage 42"}
local patterns = {"", "abcd", "abcd%d+", "^abcd$", "abcd*", "abcd+", "abcd?", "abcde-",
    "(abcd)", "()abcd()", "(%a+) hits (.-) for (%d+) damage%.", "damage %d+",
    "[%a]abcd", "[^]]abcd", "[]%]]abcd", "abcd%z", "abcd%.", "abcd%1",
    "(abcd)%1", "%babcd", "abcd%f[%a]", "abcd(", "abcd)", "abcd[", "abcd[]",
    "abcd%", "abcd[%]", "abcd**", "abcd\000.*", "\000abcd", "(.-)abcd"}
local replacements = {"replacement", "%1", "%9", "%", "\000", 17, false, {},
    function() return "replacement" end}
for _, subject in ipairs(subjects) do
    for _, pattern in ipairs(patterns) do
        record(pack(pcall(string.find, subject, pattern)))
        record(pack(pcall(string.find, subject, pattern, 1)))
        record(pack(pcall(string.find, subject, pattern, -2)))
        record(pack(pcall(string.find, subject, pattern, {}, true)))
        record(pack(pcall(string.find, subject, pattern, nil, true)))
        for _, replacement in ipairs(replacements) do
            record(pack(pcall(string.gsub, subject, pattern, replacement)))
            record(pack(pcall(string.gsub, subject, pattern, replacement, 0)))
            record(pack(pcall(string.gsub, subject, pattern, replacement, 2)))
        end
        local iterator = string.gfind(subject, pattern)
        for step = 1, 6 do
            record(pack(pcall(iterator)))
            record(pack(debug.getupvalue(iterator, 3)))
        end
    end
end
for _, bad in ipairs({false, {}, 42}) do
    record(pack(pcall(string.find, bad, "abcd%d")))
    record(pack(pcall(string.find, "abcd", bad)))
    record(pack(pcall(string.gsub, bad, "abcd%d", "x")))
    record(pack(pcall(string.gsub, "abcd", bad, "x")))
end
local seed = 12345
local atoms = {"abcd", "damage ", "%d", "%a", ".", "x", "[ab]", "()", "(abcd)", "%b()", "%1", "%"}
local suffixes = {"", "*", "+", "-", "?"}
for trial = 1, 2000 do
    seed = math.mod(seed * 48271, 2147483647)
    local a = atoms[math.mod(seed, table.getn(atoms)) + 1]
    seed = math.mod(seed * 48271, 2147483647)
    local b = atoms[math.mod(seed, table.getn(atoms)) + 1]
    local pattern = a .. suffixes[math.mod(trial, 5) + 1] .. b
    local subject = subjects[math.mod(trial, table.getn(subjects)) + 1]
    record(pack(pcall(string.find, subject, pattern)))
    record(pack(pcall(string.gsub, subject, pattern, "x")))
    record(pack(pcall(string.gfind(subject, pattern))))
end
return table.concat(records, "\n")
