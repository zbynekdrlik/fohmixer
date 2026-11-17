{
	"patcher": {
		"fileversion": 1,
		"appversion": {
			"major": 8,
			"minor": 6,
			"revision": 5,
			"architecture": "x64",
			"modernui": 1
		},
		"classnamespace": "box",
		"rect": [
			100.0,
			100.0,
			1400.0,
			900.0
		],
		"openinpresentation": 1,
		"default_fontsize": 10.0,
		"default_fontface": 0,
		"default_fontname": "Arial Bold",
		"gridsize": [
			8.0,
			8.0
		],
		"boxanimatetime": 500,
		"description": "FohMixer Listen: the track's sound to the FohMixer hub on 127.0.0.1 while the hub selects it.",
		"devicewidth": 180.0,
		"dependency_cache": [],
		"autosave": 0,
		"boxes": [
			{
				"box": {
					"id": "obj-plugin",
					"maxclass": "newobj",
					"text": "plugin~",
					"numinlets": 2,
					"numoutlets": 2,
					"outlettype": [
						"signal",
						"signal"
					],
					"patching_rect": [
						30.0,
						30.0,
						76.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-silence",
					"maxclass": "newobj",
					"text": "sig~ 0.",
					"numinlets": 1,
					"numoutlets": 1,
					"outlettype": [
						"signal"
					],
					"patching_rect": [
						30.0,
						80.0,
						76.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-plugout",
					"maxclass": "newobj",
					"text": "plugout~",
					"numinlets": 2,
					"numoutlets": 2,
					"outlettype": [
						"signal",
						"signal"
					],
					"patching_rect": [
						30.0,
						120.0,
						84.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-catch",
					"maxclass": "newobj",
					"text": "jit.catch~ 2 @mode 1 @framesize 960",
					"numinlets": 2,
					"numoutlets": 2,
					"outlettype": [
						"jit_matrix",
						""
					],
					"patching_rect": [
						200.0,
						80.0,
						220.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-net",
					"maxclass": "newobj",
					"text": "jit.net.send @ip 127.0.0.1 @port 39111",
					"numinlets": 2,
					"numoutlets": 1,
					"outlettype": [
						""
					],
					"patching_rect": [
						200.0,
						160.0,
						230.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-qmetro",
					"maxclass": "newobj",
					"text": "qmetro 10",
					"numinlets": 2,
					"numoutlets": 1,
					"outlettype": [
						"bang"
					],
					"patching_rect": [
						200.0,
						30.0,
						70.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-poll-qmetro",
					"maxclass": "newobj",
					"text": "qmetro 250",
					"numinlets": 2,
					"numoutlets": 1,
					"outlettype": [
						"bang"
					],
					"patching_rect": [
						460.0,
						30.0,
						70.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-poll-request",
					"maxclass": "newobj",
					"text": "dict.pack url: http_method: parse_type:",
					"numinlets": 3,
					"numoutlets": 1,
					"outlettype": [
						"dictionary"
					],
					"patching_rect": [
						460.0,
						60.0,
						230.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-poll-method",
					"maxclass": "message",
					"text": "get",
					"numinlets": 2,
					"numoutlets": 1,
					"outlettype": [
						""
					],
					"patching_rect": [
						560.0,
						30.0,
						34.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-poll-parse",
					"maxclass": "message",
					"text": "json",
					"numinlets": 2,
					"numoutlets": 1,
					"outlettype": [
						""
					],
					"patching_rect": [
						600.0,
						30.0,
						38.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-poll-change",
					"maxclass": "newobj",
					"text": "change",
					"numinlets": 2,
					"numoutlets": 3,
					"outlettype": [
						"",
						"int",
						"int"
					],
					"patching_rect": [
						700.0,
						600.0,
						60.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-poll-t",
					"maxclass": "newobj",
					"text": "t b l",
					"numinlets": 1,
					"numoutlets": 2,
					"outlettype": [
						"bang",
						""
					],
					"patching_rect": [
						780.0,
						640.0,
						50.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-poll-url",
					"maxclass": "newobj",
					"text": "sprintf http://127.0.0.1:8480/api/listen/select?device=%ld",
					"numinlets": 1,
					"numoutlets": 1,
					"outlettype": [
						""
					],
					"patching_rect": [
						780.0,
						680.0,
						380.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-poll-maxurl",
					"maxclass": "newobj",
					"text": "maxurl",
					"numinlets": 1,
					"numoutlets": 2,
					"outlettype": [
						"dictionary",
						""
					],
					"patching_rect": [
						460.0,
						90.0,
						60.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-poll-body",
					"maxclass": "newobj",
					"text": "dict.unpack body:",
					"numinlets": 1,
					"numoutlets": 2,
					"outlettype": [
						"",
						""
					],
					"patching_rect": [
						460.0,
						120.0,
						110.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-poll-on",
					"maxclass": "newobj",
					"text": "dict.unpack on:",
					"numinlets": 1,
					"numoutlets": 2,
					"outlettype": [
						"",
						""
					],
					"patching_rect": [
						460.0,
						150.0,
						100.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-poll-sel",
					"maxclass": "newobj",
					"text": "sel 1",
					"numinlets": 2,
					"numoutlets": 2,
					"outlettype": [
						"bang",
						""
					],
					"patching_rect": [
						460.0,
						165.0,
						50.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-session-random",
					"maxclass": "newobj",
					"text": "random 21000",
					"numinlets": 2,
					"numoutlets": 1,
					"outlettype": [
						"int"
					],
					"patching_rect": [
						700.0,
						600.0,
						90.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-session-id",
					"maxclass": "newobj",
					"text": "expr $i2 * 100000 + $i1 % 100000 + 1",
					"numinlets": 2,
					"numoutlets": 1,
					"outlettype": [
						""
					],
					"patching_rect": [
						700.0,
						640.0,
						220.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-select-t",
					"maxclass": "newobj",
					"text": "t b b",
					"numinlets": 1,
					"numoutlets": 2,
					"outlettype": [
						"bang",
						"bang"
					],
					"patching_rect": [
						460.0,
						180.0,
						50.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-select-on",
					"maxclass": "message",
					"text": "1",
					"numinlets": 2,
					"numoutlets": 1,
					"outlettype": [
						""
					],
					"patching_rect": [
						520.0,
						210.0,
						30.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-select-off",
					"maxclass": "message",
					"text": "0",
					"numinlets": 2,
					"numoutlets": 1,
					"outlettype": [
						""
					],
					"patching_rect": [
						600.0,
						210.0,
						30.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-select-hold",
					"maxclass": "newobj",
					"text": "delay 1000",
					"numinlets": 2,
					"numoutlets": 1,
					"outlettype": [
						"bang"
					],
					"patching_rect": [
						460.0,
						210.0,
						70.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-state",
					"maxclass": "newobj",
					"text": "change",
					"numinlets": 2,
					"numoutlets": 3,
					"outlettype": [
						"",
						"int",
						"int"
					],
					"patching_rect": [
						520.0,
						250.0,
						60.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-state-sel",
					"maxclass": "newobj",
					"text": "sel 1 0",
					"numinlets": 3,
					"numoutlets": 3,
					"outlettype": [
						"bang",
						"bang",
						""
					],
					"patching_rect": [
						520.0,
						280.0,
						60.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-start",
					"maxclass": "newobj",
					"text": "t b b",
					"numinlets": 1,
					"numoutlets": 2,
					"outlettype": [
						"bang",
						"bang"
					],
					"patching_rect": [
						520.0,
						310.0,
						50.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-run",
					"maxclass": "message",
					"text": "1",
					"numinlets": 2,
					"numoutlets": 1,
					"outlettype": [
						""
					],
					"patching_rect": [
						520.0,
						340.0,
						30.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-halt",
					"maxclass": "message",
					"text": "0",
					"numinlets": 2,
					"numoutlets": 1,
					"outlettype": [
						""
					],
					"patching_rect": [
						580.0,
						340.0,
						30.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-hello",
					"maxclass": "newobj",
					"text": "pack fohmixer_listen 1 0 v1.54.2 0",
					"numinlets": 5,
					"numoutlets": 1,
					"outlettype": [
						""
					],
					"patching_rect": [
						200.0,
						400.0,
						260.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-hello-gate",
					"maxclass": "newobj",
					"text": "gate",
					"numinlets": 2,
					"numoutlets": 1,
					"outlettype": [
						""
					],
					"patching_rect": [
						200.0,
						370.0,
						40.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-announce",
					"maxclass": "newobj",
					"text": "pack /fohmixer/listen 1 0 v1.54.2 - - - - 0 0",
					"numinlets": 10,
					"numoutlets": 1,
					"outlettype": [
						""
					],
					"patching_rect": [
						30.0,
						460.0,
						420.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-udpsend",
					"maxclass": "newobj",
					"text": "udpsend 127.0.0.1 39111",
					"numinlets": 1,
					"numoutlets": 0,
					"outlettype": [],
					"patching_rect": [
						30.0,
						500.0,
						150.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-dsp",
					"maxclass": "newobj",
					"text": "dspstate~",
					"numinlets": 1,
					"numoutlets": 4,
					"outlettype": [
						"int",
						"float",
						"int",
						"int"
					],
					"patching_rect": [
						620.0,
						400.0,
						70.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-title",
					"maxclass": "comment",
					"numinlets": 1,
					"numoutlets": 0,
					"patching_rect": [
						350.0,
						540.0,
						100.0,
						18.0
					],
					"presentation": 1,
					"presentation_rect": [
						8.0,
						10.0,
						100.0,
						18.0
					],
					"text": "FohMixer Listen"
				}
			},
			{
				"box": {
					"id": "obj-version",
					"maxclass": "comment",
					"numinlets": 1,
					"numoutlets": 0,
					"patching_rect": [
						460.0,
						540.0,
						60.0,
						18.0
					],
					"presentation": 1,
					"presentation_rect": [
						108.0,
						10.0,
						60.0,
						18.0
					],
					"text": "v1.54.2",
					"varname": "version"
				}
			},
			{
				"box": {
					"id": "obj-label-sending",
					"maxclass": "comment",
					"numinlets": 1,
					"numoutlets": 0,
					"patching_rect": [
						130.0,
						540.0,
						72.0,
						18.0
					],
					"presentation": 1,
					"presentation_rect": [
						8.0,
						34.0,
						72.0,
						18.0
					],
					"text": "Sending"
				}
			},
			{
				"box": {
					"id": "obj-sending",
					"maxclass": "number",
					"numinlets": 1,
					"numoutlets": 2,
					"outlettype": [
						"",
						"bang"
					],
					"patching_rect": [
						130.0,
						570.0,
						50.0,
						18.0
					],
					"presentation": 1,
					"presentation_rect": [
						88.0,
						34.0,
						50.0,
						18.0
					],
					"ignoreclick": 1,
					"parameter_enable": 0
				}
			},
			{
				"box": {
					"id": "obj-thisdevice",
					"maxclass": "newobj",
					"text": "live.thisdevice",
					"numinlets": 1,
					"numoutlets": 3,
					"outlettype": [
						"bang",
						"int",
						"int"
					],
					"patching_rect": [
						420.0,
						620.0,
						140.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-id-metro",
					"maxclass": "newobj",
					"text": "metro 1000",
					"numinlets": 2,
					"numoutlets": 1,
					"outlettype": [
						"bang"
					],
					"patching_rect": [
						540.0,
						660.0,
						100.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-id-defer",
					"maxclass": "newobj",
					"text": "deferlow",
					"numinlets": 1,
					"numoutlets": 1,
					"outlettype": [
						""
					],
					"patching_rect": [
						420.0,
						700.0,
						84.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-id-refresh",
					"maxclass": "newobj",
					"text": "t b b b b b b b b",
					"numinlets": 1,
					"numoutlets": 8,
					"outlettype": [
						"bang",
						"bang",
						"bang",
						"bang",
						"bang",
						"bang",
						"bang",
						"bang"
					],
					"patching_rect": [
						420.0,
						740.0,
						140.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-id-none",
					"maxclass": "message",
					"text": "-",
					"numinlets": 2,
					"numoutlets": 1,
					"outlettype": [
						""
					],
					"patching_rect": [
						820.0,
						780.0,
						28.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-id-path",
					"maxclass": "message",
					"text": "path this_device",
					"numinlets": 2,
					"numoutlets": 1,
					"outlettype": [
						""
					],
					"patching_rect": [
						720.0,
						820.0,
						148.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-id-live-path",
					"maxclass": "newobj",
					"text": "live.path",
					"numinlets": 1,
					"numoutlets": 3,
					"outlettype": [
						"",
						"",
						""
					],
					"patching_rect": [
						720.0,
						860.0,
						92.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-id-route",
					"maxclass": "newobj",
					"text": "route id",
					"numinlets": 2,
					"numoutlets": 2,
					"outlettype": [
						"",
						""
					],
					"patching_rect": [
						720.0,
						900.0,
						84.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-parent1-path",
					"maxclass": "message",
					"text": "path this_device canonical_parent",
					"numinlets": 2,
					"numoutlets": 1,
					"outlettype": [
						""
					],
					"patching_rect": [
						420.0,
						960.0,
						284.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-parent1-live-path",
					"maxclass": "newobj",
					"text": "live.path",
					"numinlets": 1,
					"numoutlets": 3,
					"outlettype": [
						"",
						"",
						""
					],
					"patching_rect": [
						420.0,
						1000.0,
						92.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-parent1-route-id",
					"maxclass": "newobj",
					"text": "route id",
					"numinlets": 2,
					"numoutlets": 2,
					"outlettype": [
						"",
						""
					],
					"patching_rect": [
						420.0,
						1040.0,
						84.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-parent1-sel",
					"maxclass": "newobj",
					"text": "sel 0",
					"numinlets": 2,
					"numoutlets": 2,
					"outlettype": [
						"bang",
						""
					],
					"patching_rect": [
						420.0,
						1080.0,
						60.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-parent1-t",
					"maxclass": "newobj",
					"text": "t b i",
					"numinlets": 1,
					"numoutlets": 2,
					"outlettype": [
						"bang",
						"int"
					],
					"patching_rect": [
						480.0,
						1120.0,
						60.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-parent1-prepend",
					"maxclass": "newobj",
					"text": "prepend id",
					"numinlets": 1,
					"numoutlets": 1,
					"outlettype": [
						""
					],
					"patching_rect": [
						540.0,
						1160.0,
						100.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-parent1-get",
					"maxclass": "message",
					"text": "get name",
					"numinlets": 2,
					"numoutlets": 1,
					"outlettype": [
						""
					],
					"patching_rect": [
						480.0,
						1160.0,
						84.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-parent1-object",
					"maxclass": "newobj",
					"text": "live.object",
					"numinlets": 2,
					"numoutlets": 1,
					"outlettype": [
						""
					],
					"patching_rect": [
						480.0,
						1200.0,
						108.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-parent1-route-name",
					"maxclass": "newobj",
					"text": "route name",
					"numinlets": 2,
					"numoutlets": 2,
					"outlettype": [
						"",
						""
					],
					"patching_rect": [
						480.0,
						1240.0,
						100.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-parent1-reg",
					"maxclass": "newobj",
					"text": "zl reg",
					"numinlets": 2,
					"numoutlets": 2,
					"outlettype": [
						"",
						""
					],
					"patching_rect": [
						420.0,
						1280.0,
						68.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-parent1-symbol",
					"maxclass": "newobj",
					"text": "tosymbol",
					"numinlets": 1,
					"numoutlets": 1,
					"outlettype": [
						""
					],
					"patching_rect": [
						420.0,
						1320.0,
						84.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-parent2-path",
					"maxclass": "message",
					"text": "path this_device canonical_parent canonical_parent",
					"numinlets": 2,
					"numoutlets": 1,
					"outlettype": [
						""
					],
					"patching_rect": [
						680.0,
						960.0,
						420.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-parent2-live-path",
					"maxclass": "newobj",
					"text": "live.path",
					"numinlets": 1,
					"numoutlets": 3,
					"outlettype": [
						"",
						"",
						""
					],
					"patching_rect": [
						680.0,
						1000.0,
						92.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-parent2-route-id",
					"maxclass": "newobj",
					"text": "route id",
					"numinlets": 2,
					"numoutlets": 2,
					"outlettype": [
						"",
						""
					],
					"patching_rect": [
						680.0,
						1040.0,
						84.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-parent2-sel",
					"maxclass": "newobj",
					"text": "sel 0",
					"numinlets": 2,
					"numoutlets": 2,
					"outlettype": [
						"bang",
						""
					],
					"patching_rect": [
						680.0,
						1080.0,
						60.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-parent2-t",
					"maxclass": "newobj",
					"text": "t b i",
					"numinlets": 1,
					"numoutlets": 2,
					"outlettype": [
						"bang",
						"int"
					],
					"patching_rect": [
						740.0,
						1120.0,
						60.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-parent2-prepend",
					"maxclass": "newobj",
					"text": "prepend id",
					"numinlets": 1,
					"numoutlets": 1,
					"outlettype": [
						""
					],
					"patching_rect": [
						800.0,
						1160.0,
						100.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-parent2-get",
					"maxclass": "message",
					"text": "get name",
					"numinlets": 2,
					"numoutlets": 1,
					"outlettype": [
						""
					],
					"patching_rect": [
						740.0,
						1160.0,
						84.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-parent2-object",
					"maxclass": "newobj",
					"text": "live.object",
					"numinlets": 2,
					"numoutlets": 1,
					"outlettype": [
						""
					],
					"patching_rect": [
						740.0,
						1200.0,
						108.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-parent2-route-name",
					"maxclass": "newobj",
					"text": "route name",
					"numinlets": 2,
					"numoutlets": 2,
					"outlettype": [
						"",
						""
					],
					"patching_rect": [
						740.0,
						1240.0,
						100.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-parent2-reg",
					"maxclass": "newobj",
					"text": "zl reg",
					"numinlets": 2,
					"numoutlets": 2,
					"outlettype": [
						"",
						""
					],
					"patching_rect": [
						680.0,
						1280.0,
						68.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-parent2-symbol",
					"maxclass": "newobj",
					"text": "tosymbol",
					"numinlets": 1,
					"numoutlets": 1,
					"outlettype": [
						""
					],
					"patching_rect": [
						680.0,
						1320.0,
						84.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-parent3-path",
					"maxclass": "message",
					"text": "path this_device canonical_parent canonical_parent canonical_parent",
					"numinlets": 2,
					"numoutlets": 1,
					"outlettype": [
						""
					],
					"patching_rect": [
						940.0,
						960.0,
						556.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-parent3-live-path",
					"maxclass": "newobj",
					"text": "live.path",
					"numinlets": 1,
					"numoutlets": 3,
					"outlettype": [
						"",
						"",
						""
					],
					"patching_rect": [
						940.0,
						1000.0,
						92.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-parent3-route-id",
					"maxclass": "newobj",
					"text": "route id",
					"numinlets": 2,
					"numoutlets": 2,
					"outlettype": [
						"",
						""
					],
					"patching_rect": [
						940.0,
						1040.0,
						84.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-parent3-sel",
					"maxclass": "newobj",
					"text": "sel 0",
					"numinlets": 2,
					"numoutlets": 2,
					"outlettype": [
						"bang",
						""
					],
					"patching_rect": [
						940.0,
						1080.0,
						60.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-parent3-t",
					"maxclass": "newobj",
					"text": "t b i",
					"numinlets": 1,
					"numoutlets": 2,
					"outlettype": [
						"bang",
						"int"
					],
					"patching_rect": [
						1000.0,
						1120.0,
						60.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-parent3-prepend",
					"maxclass": "newobj",
					"text": "prepend id",
					"numinlets": 1,
					"numoutlets": 1,
					"outlettype": [
						""
					],
					"patching_rect": [
						1060.0,
						1160.0,
						100.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-parent3-get",
					"maxclass": "message",
					"text": "get name",
					"numinlets": 2,
					"numoutlets": 1,
					"outlettype": [
						""
					],
					"patching_rect": [
						1000.0,
						1160.0,
						84.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-parent3-object",
					"maxclass": "newobj",
					"text": "live.object",
					"numinlets": 2,
					"numoutlets": 1,
					"outlettype": [
						""
					],
					"patching_rect": [
						1000.0,
						1200.0,
						108.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-parent3-route-name",
					"maxclass": "newobj",
					"text": "route name",
					"numinlets": 2,
					"numoutlets": 2,
					"outlettype": [
						"",
						""
					],
					"patching_rect": [
						1000.0,
						1240.0,
						100.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-parent3-reg",
					"maxclass": "newobj",
					"text": "zl reg",
					"numinlets": 2,
					"numoutlets": 2,
					"outlettype": [
						"",
						""
					],
					"patching_rect": [
						940.0,
						1280.0,
						68.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-parent3-symbol",
					"maxclass": "newobj",
					"text": "tosymbol",
					"numinlets": 1,
					"numoutlets": 1,
					"outlettype": [
						""
					],
					"patching_rect": [
						940.0,
						1320.0,
						84.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-parent4-path",
					"maxclass": "message",
					"text": "path this_device canonical_parent canonical_parent canonical_parent canonical_parent",
					"numinlets": 2,
					"numoutlets": 1,
					"outlettype": [
						""
					],
					"patching_rect": [
						1200.0,
						960.0,
						692.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-parent4-live-path",
					"maxclass": "newobj",
					"text": "live.path",
					"numinlets": 1,
					"numoutlets": 3,
					"outlettype": [
						"",
						"",
						""
					],
					"patching_rect": [
						1200.0,
						1000.0,
						92.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-parent4-route-id",
					"maxclass": "newobj",
					"text": "route id",
					"numinlets": 2,
					"numoutlets": 2,
					"outlettype": [
						"",
						""
					],
					"patching_rect": [
						1200.0,
						1040.0,
						84.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-parent4-sel",
					"maxclass": "newobj",
					"text": "sel 0",
					"numinlets": 2,
					"numoutlets": 2,
					"outlettype": [
						"bang",
						""
					],
					"patching_rect": [
						1200.0,
						1080.0,
						60.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-parent4-t",
					"maxclass": "newobj",
					"text": "t b i",
					"numinlets": 1,
					"numoutlets": 2,
					"outlettype": [
						"bang",
						"int"
					],
					"patching_rect": [
						1260.0,
						1120.0,
						60.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-parent4-prepend",
					"maxclass": "newobj",
					"text": "prepend id",
					"numinlets": 1,
					"numoutlets": 1,
					"outlettype": [
						""
					],
					"patching_rect": [
						1320.0,
						1160.0,
						100.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-parent4-get",
					"maxclass": "message",
					"text": "get name",
					"numinlets": 2,
					"numoutlets": 1,
					"outlettype": [
						""
					],
					"patching_rect": [
						1260.0,
						1160.0,
						84.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-parent4-object",
					"maxclass": "newobj",
					"text": "live.object",
					"numinlets": 2,
					"numoutlets": 1,
					"outlettype": [
						""
					],
					"patching_rect": [
						1260.0,
						1200.0,
						108.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-parent4-route-name",
					"maxclass": "newobj",
					"text": "route name",
					"numinlets": 2,
					"numoutlets": 2,
					"outlettype": [
						"",
						""
					],
					"patching_rect": [
						1260.0,
						1240.0,
						100.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-parent4-reg",
					"maxclass": "newobj",
					"text": "zl reg",
					"numinlets": 2,
					"numoutlets": 2,
					"outlettype": [
						"",
						""
					],
					"patching_rect": [
						1200.0,
						1280.0,
						68.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-parent4-symbol",
					"maxclass": "newobj",
					"text": "tosymbol",
					"numinlets": 1,
					"numoutlets": 1,
					"outlettype": [
						""
					],
					"patching_rect": [
						1200.0,
						1320.0,
						84.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-select-defer",
					"maxclass": "newobj",
					"text": "deferlow",
					"numinlets": 1,
					"numoutlets": 1,
					"outlettype": [
						""
					],
					"patching_rect": [
						536.0,
						210.0,
						58.0,
						22.0
					]
				}
			}
		],
		"lines": [
			{
				"patchline": {
					"source": [
						"obj-silence",
						0
					],
					"destination": [
						"obj-plugout",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-silence",
						0
					],
					"destination": [
						"obj-plugout",
						1
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-plugin",
						0
					],
					"destination": [
						"obj-catch",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-plugin",
						1
					],
					"destination": [
						"obj-catch",
						1
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-qmetro",
						0
					],
					"destination": [
						"obj-catch",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-catch",
						0
					],
					"destination": [
						"obj-net",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-hello",
						0
					],
					"destination": [
						"obj-net",
						1
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-thisdevice",
						0
					],
					"destination": [
						"obj-poll-method",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-thisdevice",
						0
					],
					"destination": [
						"obj-poll-parse",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-poll-method",
						0
					],
					"destination": [
						"obj-poll-request",
						1
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-session-id",
						0
					],
					"destination": [
						"obj-poll-change",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-poll-change",
						0
					],
					"destination": [
						"obj-poll-t",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-poll-t",
						1
					],
					"destination": [
						"obj-poll-url",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-poll-t",
						0
					],
					"destination": [
						"obj-poll-qmetro",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-poll-parse",
						0
					],
					"destination": [
						"obj-poll-request",
						2
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-poll-url",
						0
					],
					"destination": [
						"obj-poll-request",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-poll-qmetro",
						0
					],
					"destination": [
						"obj-poll-request",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-poll-request",
						0
					],
					"destination": [
						"obj-poll-maxurl",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-poll-maxurl",
						0
					],
					"destination": [
						"obj-poll-body",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-poll-body",
						0
					],
					"destination": [
						"obj-poll-on",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-poll-on",
						0
					],
					"destination": [
						"obj-poll-sel",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-poll-sel",
						0
					],
					"destination": [
						"obj-select-t",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-poll-sel",
						1
					],
					"destination": [
						"obj-select-off",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-select-t",
						1
					],
					"destination": [
						"obj-select-hold",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-select-t",
						0
					],
					"destination": [
						"obj-select-on",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-select-on",
						0
					],
					"destination": [
						"obj-state",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-select-off",
						0
					],
					"destination": [
						"obj-state",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-state",
						0
					],
					"destination": [
						"obj-state-sel",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-state",
						0
					],
					"destination": [
						"obj-announce",
						9
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-state",
						0
					],
					"destination": [
						"obj-sending",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-state",
						0
					],
					"destination": [
						"obj-hello-gate",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-state-sel",
						0
					],
					"destination": [
						"obj-start",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-state-sel",
						1
					],
					"destination": [
						"obj-halt",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-start",
						1
					],
					"destination": [
						"obj-hello",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-start",
						0
					],
					"destination": [
						"obj-run",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-run",
						0
					],
					"destination": [
						"obj-qmetro",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-halt",
						0
					],
					"destination": [
						"obj-qmetro",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-thisdevice",
						0
					],
					"destination": [
						"obj-session-random",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-session-random",
						0
					],
					"destination": [
						"obj-session-id",
						1
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-id-route",
						0
					],
					"destination": [
						"obj-session-id",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-session-id",
						0
					],
					"destination": [
						"obj-announce",
						2
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-session-id",
						0
					],
					"destination": [
						"obj-hello",
						2
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent1-symbol",
						0
					],
					"destination": [
						"obj-announce",
						4
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent2-symbol",
						0
					],
					"destination": [
						"obj-announce",
						5
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent3-symbol",
						0
					],
					"destination": [
						"obj-announce",
						6
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent4-symbol",
						0
					],
					"destination": [
						"obj-announce",
						7
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-id-refresh",
						1
					],
					"destination": [
						"obj-dsp",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-dsp",
						1
					],
					"destination": [
						"obj-announce",
						8
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-dsp",
						1
					],
					"destination": [
						"obj-hello",
						4
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-id-refresh",
						0
					],
					"destination": [
						"obj-announce",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-id-refresh",
						0
					],
					"destination": [
						"obj-hello-gate",
						1
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-hello-gate",
						0
					],
					"destination": [
						"obj-hello",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-announce",
						0
					],
					"destination": [
						"obj-udpsend",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-thisdevice",
						0
					],
					"destination": [
						"obj-id-defer",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-thisdevice",
						0
					],
					"destination": [
						"obj-id-metro",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-id-metro",
						0
					],
					"destination": [
						"obj-id-defer",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-id-defer",
						0
					],
					"destination": [
						"obj-id-refresh",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-id-refresh",
						7
					],
					"destination": [
						"obj-id-none",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-id-refresh",
						6
					],
					"destination": [
						"obj-id-path",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-id-path",
						0
					],
					"destination": [
						"obj-id-live-path",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-id-live-path",
						0
					],
					"destination": [
						"obj-id-route",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-id-refresh",
						5
					],
					"destination": [
						"obj-parent1-path",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent1-path",
						0
					],
					"destination": [
						"obj-parent1-live-path",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent1-live-path",
						0
					],
					"destination": [
						"obj-parent1-route-id",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent1-route-id",
						0
					],
					"destination": [
						"obj-parent1-sel",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent1-sel",
						1
					],
					"destination": [
						"obj-parent1-t",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent1-t",
						1
					],
					"destination": [
						"obj-parent1-prepend",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent1-prepend",
						0
					],
					"destination": [
						"obj-parent1-object",
						1
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent1-t",
						0
					],
					"destination": [
						"obj-parent1-get",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent1-get",
						0
					],
					"destination": [
						"obj-parent1-object",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent1-object",
						0
					],
					"destination": [
						"obj-parent1-route-name",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent1-route-name",
						0
					],
					"destination": [
						"obj-parent1-reg",
						1
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-id-none",
						0
					],
					"destination": [
						"obj-parent1-reg",
						1
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-id-refresh",
						1
					],
					"destination": [
						"obj-parent1-reg",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent1-reg",
						0
					],
					"destination": [
						"obj-parent1-symbol",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-id-refresh",
						4
					],
					"destination": [
						"obj-parent2-path",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent2-path",
						0
					],
					"destination": [
						"obj-parent2-live-path",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent2-live-path",
						0
					],
					"destination": [
						"obj-parent2-route-id",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent2-route-id",
						0
					],
					"destination": [
						"obj-parent2-sel",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent2-sel",
						1
					],
					"destination": [
						"obj-parent2-t",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent2-t",
						1
					],
					"destination": [
						"obj-parent2-prepend",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent2-prepend",
						0
					],
					"destination": [
						"obj-parent2-object",
						1
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent2-t",
						0
					],
					"destination": [
						"obj-parent2-get",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent2-get",
						0
					],
					"destination": [
						"obj-parent2-object",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent2-object",
						0
					],
					"destination": [
						"obj-parent2-route-name",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent2-route-name",
						0
					],
					"destination": [
						"obj-parent2-reg",
						1
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-id-none",
						0
					],
					"destination": [
						"obj-parent2-reg",
						1
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-id-refresh",
						1
					],
					"destination": [
						"obj-parent2-reg",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent2-reg",
						0
					],
					"destination": [
						"obj-parent2-symbol",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-id-refresh",
						3
					],
					"destination": [
						"obj-parent3-path",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent3-path",
						0
					],
					"destination": [
						"obj-parent3-live-path",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent3-live-path",
						0
					],
					"destination": [
						"obj-parent3-route-id",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent3-route-id",
						0
					],
					"destination": [
						"obj-parent3-sel",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent3-sel",
						1
					],
					"destination": [
						"obj-parent3-t",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent3-t",
						1
					],
					"destination": [
						"obj-parent3-prepend",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent3-prepend",
						0
					],
					"destination": [
						"obj-parent3-object",
						1
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent3-t",
						0
					],
					"destination": [
						"obj-parent3-get",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent3-get",
						0
					],
					"destination": [
						"obj-parent3-object",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent3-object",
						0
					],
					"destination": [
						"obj-parent3-route-name",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent3-route-name",
						0
					],
					"destination": [
						"obj-parent3-reg",
						1
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-id-none",
						0
					],
					"destination": [
						"obj-parent3-reg",
						1
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-id-refresh",
						1
					],
					"destination": [
						"obj-parent3-reg",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent3-reg",
						0
					],
					"destination": [
						"obj-parent3-symbol",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-id-refresh",
						2
					],
					"destination": [
						"obj-parent4-path",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent4-path",
						0
					],
					"destination": [
						"obj-parent4-live-path",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent4-live-path",
						0
					],
					"destination": [
						"obj-parent4-route-id",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent4-route-id",
						0
					],
					"destination": [
						"obj-parent4-sel",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent4-sel",
						1
					],
					"destination": [
						"obj-parent4-t",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent4-t",
						1
					],
					"destination": [
						"obj-parent4-prepend",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent4-prepend",
						0
					],
					"destination": [
						"obj-parent4-object",
						1
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent4-t",
						0
					],
					"destination": [
						"obj-parent4-get",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent4-get",
						0
					],
					"destination": [
						"obj-parent4-object",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent4-object",
						0
					],
					"destination": [
						"obj-parent4-route-name",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent4-route-name",
						0
					],
					"destination": [
						"obj-parent4-reg",
						1
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-id-none",
						0
					],
					"destination": [
						"obj-parent4-reg",
						1
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-id-refresh",
						1
					],
					"destination": [
						"obj-parent4-reg",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent4-reg",
						0
					],
					"destination": [
						"obj-parent4-symbol",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-select-hold",
						0
					],
					"destination": [
						"obj-select-defer",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-select-defer",
						0
					],
					"destination": [
						"obj-select-off",
						0
					]
				}
			}
		]
	}
}
