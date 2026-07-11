-- WoWTranslate_API.lua
-- DLL communication via the UnitXP("WoWTranslate", ...) bridge.
-- Handles async batch translation requests and polling.
--
-- Backend: the DLL drives Apple's on-device Translation.framework. It is NOT an
-- HTTP/cloud translator, so there is no rate limiting; failures are per-pair
-- (UnsupportedPair / LanguagePackMissing) or transient (Timeout).
--
-- Wire contract:
--   ping            -> "pong"
--   translate_async(reqId, text, fromLang, toLang) -> "ok" | "error|<reason>"
--   poll            -> "" | one-or-more records, record-separated by \x1E,
--                      each "reqId \x1F translation \x1F error" (\x1F = field sep)
-- The DLL coalesces queued requests into one batched translation, and poll
-- returns every ready result at once, so a burst of chat surfaces in one tick.

WoWTranslate_API = {}

-- Internal state
local pendingRequests = {}
local pendingTexts = {}        -- text -> requestId; deduplicates identical in-flight requests
local unsupportedPairs = {}    -- "from>to" -> true; pairs the DLL reports unsupported on-device
local dllAvailable = false
local requestCounter = 0
local pollFrame = nil          -- created once on first poll, then reused
local pollingActive = false
local pollElapsed = 0
local activePendingCount = 0
local healthCheckElapsed = 0
local HEALTH_CHECK_INTERVAL = 60  -- Re-ping DLL every 60s to detect silent failure / recovery
local lastCallbackError = nil     -- last error captured from a pcall'd callback

-- Constants
local POLL_INTERVAL = 0.1        -- Poll every 100ms
local DEFAULT_MAX_PENDING = 32   -- Default in-flight cap; matches the DLL's MAX_BATCH so a full
                                 -- burst drains in one round trip. Overridable via WoWTranslateDB.maxPending.
-- Staleness ceiling: a translation that lands later than this is useless in live
-- chat (the message has scrolled away), so the request is dropped rather than
-- posted late. The worker translates one batch at a time, so this only needs to
-- exceed the DLL's single batch deadline.
local REQUEST_TIMEOUT = 25
local POLL_FIELD_SEP = string.char(31)   -- 0x1F US: separates fields within a poll record
local POLL_RECORD_SEP = string.char(30)  -- 0x1E RS: separates records in one poll response

-- In-flight cap. Higher is safe because the DLL batches a burst into ~one
-- round trip; raising it lets bursts queue instead of being silently dropped.
local function GetMaxPending()
    local v = WoWTranslateDB and WoWTranslateDB.maxPending
    if type(v) == "number" and v >= 1 then
        return v
    end
    return DEFAULT_MAX_PENDING
end

-- ============================================================================
-- ERROR MAPPING
-- ============================================================================

-- Map the DLL's TranslateStatus debug strings to short user-facing text.
-- Unknown values pass through unchanged so new statuses are still legible.
local FRIENDLY_ERROR = {
    UnsupportedPair     = "language pair not supported on-device",
    LanguagePackMissing = "language pack not installed - accept the macOS download prompt",
    Timeout             = "translation timed out",
    BufferTooSmall      = "message too long to translate",
    Unavailable         = "translator unavailable",
    InvalidParams       = "invalid translation request",
    Internal            = "internal translator error",
}

function WoWTranslate_API.FriendlyError(status)
    if not status or status == "" then return "" end
    return FRIENDLY_ERROR[status] or status
end

-- ============================================================================
-- DLL STATUS FUNCTIONS
-- ============================================================================

-- Check if DLL is loaded and responding
function WoWTranslate_API.CheckDLL()
    if UnitXP then
        local success, result = pcall(function()
            return UnitXP("WoWTranslate", "ping")
        end)
        if success and result == "pong" then
            dllAvailable = true
            return true
        end
    end
    dllAvailable = false
    return false
end

-- Get DLL status
function WoWTranslate_API.IsAvailable()
    return dllAvailable
end

-- ============================================================================
-- DEMAND-BASED POLLING HELPERS
-- ============================================================================

-- Called when a new request is queued
local function OnRequestQueued()
    activePendingCount = activePendingCount + 1
    if not pollingActive then
        WoWTranslate_API.StartPolling()
    end
end

-- Called when a request completes or times out
local function OnRequestCompleted()
    activePendingCount = activePendingCount - 1
    if activePendingCount <= 0 then
        activePendingCount = 0
        WoWTranslate_API.StopPolling()
    end
end

-- ============================================================================
-- TRANSLATION FUNCTIONS
-- ============================================================================

-- Inspect the DLL's synchronous reply to translate_async. Returns true if the
-- request was queued ("ok"); otherwise returns false plus the parsed reason and
-- flips dllAvailable off when the worker channel is closed.
local function DispatchOk(result)
    if result == "ok" then
        return true
    end
    local reason = result or "DLL rejected request"
    if result then
        local bar = string.find(result, "|", 1, true)
        if bar then reason = string.sub(result, bar + 1) end
    end
    if result == "error|queue closed" then
        dllAvailable = false  -- worker died; the 60s health-check ping will recover it
    end
    return false, reason
end

-- Request an async translation
-- callback(translation, error) will be called when complete
function WoWTranslate_API.Translate(text, callback, fromLang)
    if not dllAvailable then
        if callback then
            callback(nil, "DLL not available")
        end
        return false
    end

    if not text or text == "" then
        if callback then
            callback(nil, "Empty text")
        end
        return false
    end

    fromLang = fromLang or "zh"
    local toLang = WoWTranslateDB and WoWTranslateDB.incomingToLang or "en"

    -- Skip language pairs the DLL has reported unsupported on-device: retrying
    -- them on every message would fail forever and spam the error line. Showing
    -- the original (return false) is the right fallback.
    if unsupportedPairs[fromLang .. ">" .. toLang] then
        return false
    end

    -- Silently skip when the in-flight cap is reached; the message shows
    -- untranslated rather than queuing unboundedly.
    if activePendingCount >= GetMaxPending() then
        return false
    end

    -- Deduplicate: the same chat message fires in every chat frame at once, producing
    -- 4-5 identical requests before any result is cached.  Only the first needs to go
    -- to the DLL; the rest will get a cache hit when the first callback completes.
    if pendingTexts[text] then
        return false
    end

    -- Generate unique request ID
    requestCounter = requestCounter + 1
    local requestId = tostring(requestCounter)

    -- Store pending request (fromLang/toLang let poll negative-cache an unsupported pair)
    pendingRequests[requestId] = {
        callback = callback,
        text = text,
        fromLang = fromLang,
        toLang = toLang,
        timestamp = GetTime()
    }
    pendingTexts[text] = requestId

    local ok, result = pcall(function()
        return UnitXP("WoWTranslate", "translate_async", requestId, text, fromLang, toLang)
    end)

    if not ok then
        pendingRequests[requestId] = nil
        pendingTexts[text] = nil
        if callback then
            callback(nil, "DLL call failed: " .. tostring(result))
        end
        return false
    end

    -- Fail fast on a non-"ok" sync reply instead of waiting out the timeout.
    local queued, reason = DispatchOk(result)
    if not queued then
        pendingRequests[requestId] = nil
        pendingTexts[text] = nil
        if callback then
            callback(nil, reason)
        end
        return false
    end

    OnRequestQueued()
    return true, requestId
end

-- ============================================================================
-- POLLING SYSTEM
-- ============================================================================

-- Parse and dispatch one poll record: "reqId \x1F translation \x1F error".
local function HandlePollRecord(record)
    local firstSep = string.find(record, POLL_FIELD_SEP, 1, true)
    if not firstSep then return end

    local requestId = string.sub(record, 1, firstSep - 1)
    local remainder = string.sub(record, firstSep + 1)
    local secondSep = string.find(remainder, POLL_FIELD_SEP, 1, true)
    local translation, err
    if secondSep then
        translation = string.sub(remainder, 1, secondSep - 1)
        err = string.sub(remainder, secondSep + 1)
    else
        translation = remainder
        err = ""
    end

    local req = pendingRequests[requestId]
    if not req then return end
    pendingRequests[requestId] = nil
    pendingTexts[req.text] = nil
    OnRequestCompleted()

    if not req.callback then return end

    -- pcall prevents a callback error from disabling the pollFrame OnUpdate.
    -- Capture errors so /wt status can surface them for diagnosis.
    if err and err ~= "" then
        -- Permanent on-device failure: stop retrying this pair so it doesn't
        -- fail (and spam) on every future message.
        if err == "UnsupportedPair" and req.fromLang and req.toLang then
            unsupportedPairs[req.fromLang .. ">" .. req.toLang] = true
        end
        local cbOk, cbErr = pcall(req.callback, nil, err)
        if not cbOk then lastCallbackError = tostring(cbErr) end
    else
        local cbOk, cbErr = pcall(req.callback, translation, nil)
        if not cbOk then lastCallbackError = tostring(cbErr) end
    end
end

-- Poll DLL for completed translations
local function PollTranslations()
    if not dllAvailable then return end

    local success, result = pcall(function()
        return UnitXP("WoWTranslate", "poll")
    end)

    if not success then
        -- UnitXP call itself threw — DLL is gone or broken
        dllAvailable = false
        return
    end

    if result and result ~= "" then
        -- One poll can carry several record-separated results; drain them all so
        -- a burst of completions surfaces in a single tick.
        local pos = 1
        local resultLen = string.len(result)
        while pos <= resultLen do
            local sepStart = string.find(result, POLL_RECORD_SEP, pos, true)
            if sepStart then
                HandlePollRecord(string.sub(result, pos, sepStart - 1))
                pos = sepStart + 1
            else
                HandlePollRecord(string.sub(result, pos))
                break
            end
        end
    end

    -- Cleanup timed-out requests (once, after draining the batch)
    local now = GetTime()
    for id, req in pairs(pendingRequests) do
        if now - req.timestamp > REQUEST_TIMEOUT then
            pendingRequests[id] = nil
            pendingTexts[req.text] = nil
            OnRequestCompleted()
            if req.callback then
                pcall(req.callback, nil, "Request timed out")  -- pcall: don't break polling on error
            end
        end
    end
end

local function PollOnUpdate()
    pollElapsed = pollElapsed + arg1
    if pollElapsed >= POLL_INTERVAL then
        pollElapsed = 0
        PollTranslations()
    end
end

-- Start the polling frame.
--
-- The frame is created once and reused. Polling is demand-based, so it starts
-- and stops with every chat burst, and WoW never destroys or collects a frame
-- once created: a CreateFrame per start leaks one frame per quiet spell for the
-- rest of the session. `pollingActive` tracks the state that the frame's
-- existence used to stand for.
function WoWTranslate_API.StartPolling()
    if pollingActive then return end

    if not pollFrame then
        pollFrame = CreateFrame("Frame")
    end
    pollElapsed = 0
    pollFrame:SetScript("OnUpdate", PollOnUpdate)
    pollingActive = true
end

-- Stop the polling frame
function WoWTranslate_API.StopPolling()
    if not pollingActive then return end

    pollFrame:SetScript("OnUpdate", nil)
    pollingActive = false
end

-- Passive health check: re-ping DLL every 60s so we detect silent failure and
-- restore dllAvailable if the DLL recovers (e.g. after a worker restart).
local healthCheckFrame = CreateFrame("Frame")
healthCheckFrame:SetScript("OnUpdate", function()
    healthCheckElapsed = healthCheckElapsed + arg1
    if healthCheckElapsed >= HEALTH_CHECK_INTERVAL then
        healthCheckElapsed = 0
        WoWTranslate_API.CheckDLL() -- always; restores dllAvailable if DLL recovers
    end
end)

-- ============================================================================
-- OUTGOING TRANSLATION (player-sent messages)
-- ============================================================================

-- Request an async outgoing translation (configurable direction, default en -> zh)
-- callback(translation, error) will be called when complete
function WoWTranslate_API.TranslateOutgoing(text, callback)
    if not dllAvailable then
        if callback then
            callback(nil, "DLL not available")
        end
        return false
    end

    if not text or text == "" then
        if callback then
            callback(nil, "Empty text")
        end
        return false
    end

    local fromLang = WoWTranslateDB and WoWTranslateDB.outgoingFromLang or "en"
    local toLang = WoWTranslateDB and WoWTranslateDB.outgoingToLang or "zh"

    if unsupportedPairs[fromLang .. ">" .. toLang] then
        if callback then callback(nil, "UnsupportedPair") end
        return false
    end

    if activePendingCount >= GetMaxPending() then
        if callback then callback(nil, "Queue full, try again shortly") end
        return false
    end

    -- Generate unique request ID with "out_" prefix to distinguish from incoming
    requestCounter = requestCounter + 1
    local requestId = "out_" .. tostring(requestCounter)

    -- Store pending request
    pendingRequests[requestId] = {
        callback = callback,
        text = text,
        fromLang = fromLang,
        toLang = toLang,
        timestamp = GetTime()
    }

    local ok, result = pcall(function()
        return UnitXP("WoWTranslate", "translate_async", requestId, text, fromLang, toLang)
    end)

    if not ok then
        pendingRequests[requestId] = nil
        if callback then
            callback(nil, "DLL call failed: " .. tostring(result))
        end
        return false
    end

    local queued, reason = DispatchOk(result)
    if not queued then
        pendingRequests[requestId] = nil
        if callback then
            callback(nil, reason)
        end
        return false
    end

    OnRequestQueued()
    return true, requestId
end

-- ============================================================================
-- RECOVERY / STATUS (kept for /wt status and /wt reset)
-- ============================================================================

-- On-device translation has no HTTP rate limit; kept as a no-op so /wt status
-- callers keep working. Returns: isLimited (always false), secondsRemaining (0).
function WoWTranslate_API.GetRateLimitInfo()
    return false, 0
end

-- Clear the unsupported-pair negative cache so a manual recovery re-attempts
-- every pair (e.g. after the user installs a language pack).
function WoWTranslate_API.ResetBackoff()
    unsupportedPairs = {}
end

-- ============================================================================
-- DEBUG FUNCTIONS
-- ============================================================================

-- Get pending request count
function WoWTranslate_API.GetPendingCount()
    local count = 0
    for _ in pairs(pendingRequests) do
        count = count + 1
    end
    return count
end

-- Get last error thrown by a callback (nil if no error yet)
function WoWTranslate_API.GetLastCallbackError()
    return lastCallbackError
end

-- Clear all pending requests (used by /wt reset for recovery)
function WoWTranslate_API.ClearPending()
    for id, req in pairs(pendingRequests) do
        if req.callback then
            pcall(req.callback, nil, "Cleared by reset")
        end
        pendingRequests[id] = nil
    end
    pendingTexts = {}
    unsupportedPairs = {}
    activePendingCount = 0
    WoWTranslate_API.StopPolling()
end

-- Get all pending request info (for debugging)
function WoWTranslate_API.GetPendingRequests()
    local info = {}
    local now = GetTime()
    for id, req in pairs(pendingRequests) do
        table.insert(info, {
            id = id,
            text = req.text,
            age = now - req.timestamp
        })
    end
    return info
end
