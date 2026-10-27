-- mpv-remote: control mpv from a phone on the local network.
-- Starts a helper server (mpv-remote-server) next to this script, and shows a pairing QR code.
-- Options: script-opts/mpv-remote.conf

local mp = require "mp"
local msg = require "mp.msg"
local utils = require "mp.utils"
local options = require "mp.options"

local opts = {
    port = 8765,
    bind = "0.0.0.0",   -- interface the server listens on
    host = "",          -- address in the QR code: empty = auto LAN IP, "tailscale" (when connected, else LAN), an IP/name, or a full URL
    tailscale_host = "", -- address to use instead while Tailscale is connected (e.g. a Tailscale Serve URL)
    token = "",         -- generated on first run
    autostart = true,
    server_path = "",   -- override the helper location (development)
    pair_seconds = 60,  -- how long the QR code stays up
    browse_roots = "",  -- folders the phone may browse, separated by "|" (default: your home folder)
}
options.read_options(opts, "mpv-remote")

local is_windows = package.config:sub(1, 1) == "\\"

local function script_dir()
    return mp.get_script_directory() or utils.split_path(mp.get_script_file())
end

local function server_path()
    if opts.server_path ~= "" then return opts.server_path end
    return utils.join_path(script_dir(), is_windows and "mpv-remote-server.exe" or "mpv-remote-server")
end

local function run(args)
    return mp.command_native({
        name = "subprocess",
        args = args,
        capture_stdout = true,
        capture_stderr = true,
        playback_only = false,
    })
end

local function server_exists()
    local info = utils.file_info(server_path())
    return info ~= nil and info.is_file
end

---------------------------------------------------------------- token

local conf_path = mp.command_native({ "expand-path", "~~/script-opts/mpv-remote.conf" })

local function ensure_dir(path)
    if is_windows then
        run({ "cmd", "/c", "mkdir", (path:gsub("/", "\\")) })
    else
        run({ "mkdir", "-p", path })
    end
end

local function save_token(token)
    local content = ""
    local f = io.open(conf_path, "r")
    if f then
        content = f:read("*a")
        f:close()
    end
    -- Only replace a token= line at the start of a line (not "#token=").
    local replaced, n = ("\n" .. content):gsub("\ntoken=[^\r\n]*", "\ntoken=" .. token, 1)
    replaced = replaced:sub(2)
    if n == 0 then
        replaced = content .. ((content == "" or content:sub(-1) == "\n") and "" or "\n") .. "token=" .. token .. "\n"
    end
    f = io.open(conf_path, "w")
    if not f then
        ensure_dir(utils.split_path(conf_path))
        f = io.open(conf_path, "w")
    end
    if not f then
        msg.warn("could not write " .. conf_path .. "; the pairing token will change every session")
        return
    end
    f:write(replaced)
    f:close()
end

local function ensure_token()
    if opts.token ~= "" then return opts.token end
    local res = run({ server_path(), "token" })
    local token = res.status == 0 and (res.stdout or ""):match("%x+")
    if not token then
        msg.error("could not generate a token: " .. (res.stderr or ""))
        return nil
    end
    opts.token = token
    save_token(token)
    return token
end

---------------------------------------------------------------- ipc + server

local function ensure_ipc()
    local current = mp.get_property("input-ipc-server", "")
    if current ~= "" then return current end
    local pid = mp.get_property_number("pid") or (os.time() % 100000)
    local path = is_windows and ("\\\\.\\pipe\\mpv-remote-" .. pid) or ("/tmp/mpv-remote-" .. pid .. ".sock")
    if not mp.set_property("input-ipc-server", path) then
        msg.error("could not enable input-ipc-server; set it in mpv.conf instead")
        return nil
    end
    return path
end

local server_handle

local function start_server()
    if server_handle then return end
    if not server_exists() then
        msg.error("helper not found: " .. server_path())
        return
    end
    local token, pipe = ensure_token(), ensure_ipc()
    if not (token and pipe) then return end
    local args = {
        server_path(), "serve",
        "--pipe", pipe,
        "--port", tostring(opts.port),
        "--token", token,
        "--bind", opts.bind,
    }
    if opts.browse_roots ~= "" then
        for root in opts.browse_roots:gmatch("[^|]+") do
            local info = utils.file_info(root)
            if not (info and info.is_dir) then msg.warn("browse_roots: not a folder: " .. root) end
        end
        args[#args + 1] = "--roots"
        args[#args + 1] = opts.browse_roots
    end
    server_handle = mp.command_native_async({
        name = "subprocess",
        args = args,
        capture_stderr = true,
        playback_only = false,
    }, function(_, res)
        server_handle = nil
        if res and res.status ~= 0 and res.status ~= nil then
            msg.warn("helper exited (" .. tostring(res.status) .. "): " .. (res.stderr or ""))
        end
    end)
end

mp.register_event("shutdown", function()
    if server_handle then mp.abort_async_command(server_handle) end
end)

---------------------------------------------------------------- marks
-- Timestamps saved from the phone, one JSON file per media file name. The current list is
-- published as user-data/mpv-remote/marks, which the helper mirrors to the phone.

local marks = {}
local marks_dir_ready = false

local function marks_path()
    local name = mp.get_property("filename")
    if not name or name == "" then return nil end
    name = name:gsub('[<>:"/\\|?*%c]', "_")
    return utils.join_path(mp.command_native({ "expand-path", "~~/mpv-remote/marks" }), name .. ".json")
end

local function publish_marks()
    mp.set_property_native("user-data/mpv-remote/marks", marks)
end

local function save_marks()
    local path = marks_path()
    if not path then return end
    if not marks_dir_ready then
        ensure_dir((utils.split_path(path)))
        marks_dir_ready = true
    end
    local f = io.open(path, "w")
    if not f then
        msg.warn("could not write " .. path)
        return
    end
    f:write(utils.format_json(marks))
    f:close()
end

local function load_marks()
    marks = {}
    local path = marks_path()
    local f = path and io.open(path, "r")
    if f then
        local data = utils.parse_json(f:read("*a") or "")
        f:close()
        if type(data) == "table" then marks = data end
    end
    publish_marks()
end

local function fmt_time(t)
    t = math.floor(t)
    local h, m, s = math.floor(t / 3600), math.floor(t % 3600 / 60), t % 60
    if h > 0 then return string.format("%d:%02d:%02d", h, m, s) end
    return string.format("%d:%02d", m, s)
end

local function commit_marks()
    table.sort(marks, function(a, b) return a.t < b.t end)
    save_marks()
    publish_marks()
end

local function add_mark(note)
    local t = mp.get_property_number("time-pos")
    if not t then return end
    local id = 1
    for _, m in ipairs(marks) do
        if m.id >= id then id = m.id + 1 end
    end
    marks[#marks + 1] = { id = id, t = math.floor(t * 1000 + 0.5) / 1000, note = note or "" }
    commit_marks()
    mp.osd_message("Mark saved at " .. fmt_time(t))
end

local function find_mark(id)
    id = tonumber(id)
    for i, m in ipairs(marks) do
        if m.id == id then return i, m end
    end
end

mp.register_script_message("mpv-remote-mark-add", add_mark)
mp.register_script_message("mpv-remote-mark-delete", function(id)
    local i = find_mark(id)
    if i then
        table.remove(marks, i)
        commit_marks()
    end
end)
mp.register_script_message("mpv-remote-mark-note", function(id, note)
    local _, m = find_mark(id)
    if m then
        m.note = note or ""
        commit_marks()
    end
end)
mp.add_key_binding(nil, "mark", function() add_mark("") end) -- bind a key in input.conf if wanted

mp.register_event("file-loaded", load_marks)
mp.register_event("end-file", function()
    marks = {}
    publish_marks()
end)

---------------------------------------------------------------- pairing overlay

local overlay = mp.create_osd_overlay("ass-events")
local pairing = nil -- { url = ..., rows = {...} } while shown
local pair_timer

local function render_pairing()
    if not pairing then return end
    local w, h = mp.get_osd_size()
    if not w or w == 0 then return end
    local rows, n = pairing.rows, #pairing.rows
    local quiet = 4
    local cell = math.max(2, math.floor(math.min(w, h) * 0.6 / (n + 2 * quiet)))
    local total = cell * (n + 2 * quiet)
    local x0 = math.floor((w - total) / 2)
    local y0 = math.max(8, math.floor((h - total) / 2) - math.floor(h / 24))

    local function rect(x, y, rw, rh)
        return string.format("m %d %d l %d %d l %d %d l %d %d ", x, y, x + rw, y, x + rw, y + rh, x, y + rh)
    end

    local dark = {}
    for r, line in ipairs(rows) do
        local c = 1
        while c <= n do
            if line:sub(c, c) == "1" then
                local start = c
                while c <= n and line:sub(c, c) == "1" do c = c + 1 end
                dark[#dark + 1] = rect(x0 + (quiet + start - 1) * cell, y0 + (quiet + r - 1) * cell, (c - start) * cell, cell)
            else
                c = c + 1
            end
        end
    end

    local fs = math.max(14, math.floor(h / 32))
    local ass = {
        "{\\an7\\pos(0,0)\\1c&HFFFFFF&\\bord0\\shad0\\p1}" .. rect(x0, y0, total, total) .. "{\\p0}",
        "{\\an7\\pos(0,0)\\1c&H000000&\\bord0\\shad0\\p1}" .. table.concat(dark) .. "{\\p0}",
        string.format("{\\an8\\pos(%d,%d)\\fs%d\\bord2\\1c&HFFFFFF&\\3c&H000000&}Scan with your phone (same Wi-Fi)\\N{\\fs%d}%s",
            math.floor(w / 2), y0 + total + 6, fs, math.floor(fs * 0.75), pairing.url:gsub("%?t=.*", "?t=…")),
    }
    overlay.res_x, overlay.res_y = w, h
    overlay.data = table.concat(ass, "\n")
    overlay:update()
end

local function hide_pairing()
    pairing = nil
    overlay:remove()
    if pair_timer then pair_timer:kill() end
end

local function show_pairing()
    if not server_exists() then
        mp.osd_message("mpv-remote: helper not found at " .. server_path(), 5)
        return
    end
    start_server()
    local token = ensure_token()
    if not token then return end
    local args = { server_path(), "pair", "--port", tostring(opts.port), "--token", token }
    if opts.host ~= "" then
        args[#args + 1] = "--host"
        args[#args + 1] = opts.host
    end
    if opts.tailscale_host ~= "" then
        args[#args + 1] = "--tailscale-host"
        args[#args + 1] = opts.tailscale_host
    end
    local res = run(args)
    if res.status ~= 0 then
        mp.osd_message("mpv-remote: " .. ((res.stderr or ""):gsub("%s+$", "")), 6)
        return
    end
    local rows, url = {}, nil
    for line in (res.stdout or ""):gmatch("[^\r\n]+") do
        if not url then url = line else rows[#rows + 1] = line end
    end
    pairing = { url = url, rows = rows }
    msg.info("pairing URL: " .. url)
    render_pairing()
    if pair_timer then pair_timer:kill() end
    pair_timer = mp.add_timeout(opts.pair_seconds, hide_pairing)
end

local function toggle_pairing()
    if pairing then hide_pairing() else show_pairing() end
end

-- Sent by the helper whenever a phone connects: no need to keep the QR up once it has been scanned.
mp.register_script_message("mpv-remote-connected", function()
    if pairing then
        hide_pairing()
        mp.osd_message("Phone connected", 3)
    end
end)

mp.observe_property("osd-dimensions", "native", function() render_pairing() end)
mp.add_key_binding("Ctrl+r", "pair", toggle_pairing)
mp.register_script_message("pair", toggle_pairing)

if opts.autostart then start_server() end
