pub const TRANSCODING_RESOLVE: &str =
    call_lua_macros::lua_script!("sc_methods/transcoding_resolve.lua");

pub const GET_TRACK: &str = call_lua_macros::lua_script!("sc_methods/get_track.lua");

pub const PROGRESSIVE_DOWNLOAD: &str =
    call_lua_macros::lua_script!("sc_methods/progressive_download.lua");

pub const HLS_DOWNLOAD: &str = call_lua_macros::lua_script!("sc_methods/hls_download.lua");

pub const HLS_DECRYPT: &str = call_lua_macros::lua_script!("sc_methods/hls_decrypt.lua");
