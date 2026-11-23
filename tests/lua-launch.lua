-- Run with: luajit tests/lua-launch.lua
local directory = assert(debug.getinfo(1, "S").source:gsub("^@", ""):match("^(.*[/\\])"))
local actions, layers, writes = {}, {}, {}
local focused_label = "source"
rex = {
    action = function(action) actions[action.name] = action end,
    json = { encode = function(config)
        assert(config.nvim == "/test/nvim" and config.main_action == ":open:")
        assert(config.bin == nil, "Launcher-only options must not reach the binary config")
        return '{"nvim":"/test/nvim","main_action":":open:"}'
    end },
    layout = { block = function(block) return { block = block } end },
    block = { call = function(creator, method, args)
        assert(creator == "com.superlogical.terminal" and args.block_id == "source")
        if method == "pwd" then return { pwd = "file:///tmp/source%20directory" } end
        assert(method == "write")
        writes[#writes + 1] = args.data
        return {}
    end },
    session = {
        list_blocks = function() return { blocks = { { block_id = "source", label = focused_label } } } end,
        view = function() return { windows = { { layers = { {
            kind = "tiled", layout = { block_id = "source", rect = { 0, 0, 1, 1 } },
        } } } } } end,
        new_layer = function(layer) layers[#layers + 1] = layer; return {} end,
    },
}
dofile(directory .. "../rexflash.lua") { bin = "/test/rexflash", nvim = "/test/nvim", main_action = ":open:" }
local context = { session_id = "session", block_id = "source" }
for i, mode in ipairs({ "hint", "flash" }) do
    actions["rexflash_" .. mode].run(context)
    local options = layers[i].layout.block.options
    assert(options.shell == "none", "The binary must bypass shell initialization")
    assert(options.exit.on_completion == true and options.exit.quick_exit_threshold_ms == 0,
        "Even an immediate cancellation must close the disposable overlay")
    assert(options.command[1] == "/test/rexflash" and options.command[2] == mode)
    assert(options.command[4] == "session" and options.command[6] == "source")
    assert(options.command[#options.command] == "/tmp/source directory")
end
focused_label = "rexflash"
actions.rexflash_hint.run(context)
assert(#layers == 2 and writes[1] == "\27", "The same shortcut must cancel an existing overlay")
print("PASS: direct Rust launch, optional editor config, immediate cancellation, and source context")
