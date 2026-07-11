-- WoWTranslate_Cache.lua
-- Two-generation translation cache, persisted through SavedVariables.
--
-- Entries are keyed by raw chat lines, so a flat map grows without bound: every
-- distinct thing anyone ever says becomes a permanent entry, serialized on
-- logout and parsed back on every load. Instead, keep two generations. Writes
-- land in `cur`; once it fills, `cur` ages into `prev` and a fresh `cur` starts.
-- A read that hits `prev` promotes the entry back into `cur`, so hot keys
-- (player names, common phrases) survive rotation indefinitely while cold ones
-- fall away after two. Total entries stay below 2 * ROTATE_AT.

-- WoW replaces this global with the saved table *after* this file runs, so this
-- only covers a first-ever load; the real setup is WoWTranslate_CacheInit().
WoWTranslateCache = WoWTranslateCache or {}

local CACHE_VERSION = 2
local ROTATE_AT = 1000   -- entries in `cur` before it ages into `prev`

-- Cache statistics
local cacheHits = 0
local cacheMisses = 0

-- Entries in `cur`. Tracked incrementally because Lua 5.0 cannot size a hash
-- table without walking it.
local curCount = 0

local function CountEntries(t)
    local n = 0
    for _ in pairs(t) do
        n = n + 1
    end
    return n
end

-- Adopt whatever SavedVariables restored. Safe to call more than once: an early
-- Get/Save self-initializes against a table WoW may still be about to replace,
-- and InitializeSettings re-runs this over what actually landed.
function WoWTranslate_CacheInit()
    local saved = WoWTranslateCache
    if type(saved) ~= "table" then
        WoWTranslateCache = { v = CACHE_VERSION, cur = {}, prev = {} }
        curCount = 0
        return
    end

    if saved.v == CACHE_VERSION then
        -- Repair a half-written table rather than trusting its shape.
        if type(saved.cur) ~= "table" then saved.cur = {} end
        if type(saved.prev) ~= "table" then saved.prev = {} end
        curCount = CountEntries(saved.cur)
        return
    end

    -- Legacy: a flat text -> translation map. Adopt it wholesale as `prev`, so
    -- every old entry stays readable; the ones still in use promote into `cur`
    -- on their next hit and the rest age out within two rotations.
    WoWTranslateCache = { v = CACHE_VERSION, cur = {}, prev = saved }
    curCount = 0
end

local function EnsureInit()
    if type(WoWTranslateCache) ~= "table" or type(WoWTranslateCache.cur) ~= "table" then
        WoWTranslate_CacheInit()
    end
end

-- Write into `cur`, aging it into `prev` once it fills.
local function Insert(text, translation)
    local cache = WoWTranslateCache
    if cache.cur[text] == nil then
        curCount = curCount + 1
    end
    cache.cur[text] = translation

    if curCount >= ROTATE_AT then
        cache.prev = cache.cur
        cache.cur = {}
        curCount = 0
    end
end

-- Check if a translation exists in cache
function WoWTranslate_CacheGet(text)
    EnsureInit()
    local cache = WoWTranslateCache

    local hit = cache.cur[text]
    if hit then
        cacheHits = cacheHits + 1
        return hit, true
    end

    hit = cache.prev[text]
    if hit then
        -- Promote: a key still being read must not die at the next rotation.
        cache.prev[text] = nil
        Insert(text, hit)
        cacheHits = cacheHits + 1
        return hit, true
    end

    cacheMisses = cacheMisses + 1
    return nil, false
end

-- Save a translation to cache
function WoWTranslate_CacheSave(text, translation)
    if text and translation and text ~= "" and translation ~= "" then
        EnsureInit()
        Insert(text, translation)
        return true
    end
    return false
end

-- Get cache statistics
function WoWTranslate_CacheStats()
    EnsureInit()
    local cache = WoWTranslateCache
    local total = cacheHits + cacheMisses
    return {
        entries = CountEntries(cache.cur) + CountEntries(cache.prev),
        hits = cacheHits,
        misses = cacheMisses,
        hitRate = (total > 0) and (cacheHits / total * 100) or 0
    }
end

-- Clear the cache (use with caution)
function WoWTranslate_CacheClear()
    WoWTranslateCache = { v = CACHE_VERSION, cur = {}, prev = {} }
    curCount = 0
    cacheHits = 0
    cacheMisses = 0
end

-- Reset session statistics only
function WoWTranslate_CacheResetStats()
    cacheHits = 0
    cacheMisses = 0
end
