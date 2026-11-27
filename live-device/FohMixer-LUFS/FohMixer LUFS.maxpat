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
		"description": "FohMixer LUFS: BS.1770-4 short-term loudness and Voice, sent to the FohMixer hub on 127.0.0.1.",
		"devicewidth": 210.0,
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
					"id": "obj-snap-s",
					"maxclass": "newobj",
					"text": "snapshot~ 0",
					"numinlets": 2,
					"numoutlets": 1,
					"outlettype": [
						"float"
					],
					"patching_rect": [
						30.0,
						420.0,
						80.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-snap-v",
					"maxclass": "newobj",
					"text": "snapshot~ 0",
					"numinlets": 2,
					"numoutlets": 1,
					"outlettype": [
						"float"
					],
					"patching_rect": [
						380.0,
						420.0,
						80.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-pack",
					"maxclass": "newobj",
					"text": "pack /fohmixer/lufs 2 0 v1.60.9 - - - - -700 -700 0",
					"numinlets": 11,
					"numoutlets": 1,
					"outlettype": [
						""
					],
					"patching_rect": [
						30.0,
						460.0,
						440.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-udpsend",
					"maxclass": "newobj",
					"text": "udpsend 127.0.0.1 39110",
					"numinlets": 1,
					"numoutlets": 0,
					"patching_rect": [
						30.0,
						540.0,
						204.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-times-s",
					"maxclass": "newobj",
					"text": "* 10.",
					"numinlets": 2,
					"numoutlets": 1,
					"outlettype": [
						""
					],
					"patching_rect": [
						130.0,
						480.0,
						60.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-round-s",
					"maxclass": "newobj",
					"text": "round 1.",
					"numinlets": 2,
					"numoutlets": 1,
					"outlettype": [
						""
					],
					"patching_rect": [
						130.0,
						505.0,
						84.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-clip-s",
					"maxclass": "newobj",
					"text": "clip -700 0",
					"numinlets": 3,
					"numoutlets": 1,
					"outlettype": [
						""
					],
					"patching_rect": [
						130.0,
						530.0,
						108.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-times-v",
					"maxclass": "newobj",
					"text": "* 10.",
					"numinlets": 2,
					"numoutlets": 1,
					"outlettype": [
						""
					],
					"patching_rect": [
						440.0,
						480.0,
						60.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-round-v",
					"maxclass": "newobj",
					"text": "round 1.",
					"numinlets": 2,
					"numoutlets": 1,
					"outlettype": [
						""
					],
					"patching_rect": [
						440.0,
						505.0,
						84.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-clip-v",
					"maxclass": "newobj",
					"text": "clip -700 0",
					"numinlets": 3,
					"numoutlets": 1,
					"outlettype": [
						""
					],
					"patching_rect": [
						440.0,
						530.0,
						108.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-short",
					"maxclass": "flonum",
					"numinlets": 1,
					"numoutlets": 2,
					"patching_rect": [
						130.0,
						640.0,
						92.0,
						36.0
					],
					"presentation": 1,
					"presentation_rect": [
						8.0,
						40.0,
						92.0,
						36.0
					],
					"outlettype": [
						"",
						"bang"
					],
					"format": 6,
					"ignoreclick": 1,
					"minimum": -70.0,
					"maximum": 12.0,
					"parameter_enable": 0,
					"fontsize": 26.0,
					"fontface": 1,
					"numdecimalplaces": 1,
					"triscale": 0.0
				}
			},
			{
				"box": {
					"id": "obj-voice",
					"maxclass": "flonum",
					"numinlets": 1,
					"numoutlets": 2,
					"patching_rect": [
						240.0,
						640.0,
						92.0,
						36.0
					],
					"presentation": 1,
					"presentation_rect": [
						108.0,
						40.0,
						92.0,
						36.0
					],
					"outlettype": [
						"",
						"bang"
					],
					"format": 6,
					"ignoreclick": 1,
					"minimum": -70.0,
					"maximum": 12.0,
					"parameter_enable": 0,
					"fontsize": 26.0,
					"fontface": 1,
					"numdecimalplaces": 1,
					"triscale": 0.0
				}
			},
			{
				"box": {
					"id": "obj-caption",
					"maxclass": "comment",
					"numinlets": 1,
					"numoutlets": 0,
					"patching_rect": [
						350.0,
						640.0,
						60.0,
						20.0
					],
					"presentation": 1,
					"presentation_rect": [
						8.0,
						2.0,
						60.0,
						20.0
					],
					"text": "LUFS",
					"fontsize": 14.0,
					"fontface": 1
				}
			},
			{
				"box": {
					"id": "obj-label-l",
					"maxclass": "comment",
					"numinlets": 1,
					"numoutlets": 0,
					"patching_rect": [
						130.0,
						600.0,
						92.0,
						18.0
					],
					"presentation": 1,
					"presentation_rect": [
						8.0,
						22.0,
						92.0,
						18.0
					],
					"text": "Short-term",
					"fontsize": 11.0
				}
			},
			{
				"box": {
					"id": "obj-label-r",
					"maxclass": "comment",
					"numinlets": 1,
					"numoutlets": 0,
					"patching_rect": [
						240.0,
						600.0,
						92.0,
						18.0
					],
					"presentation": 1,
					"presentation_rect": [
						108.0,
						22.0,
						92.0,
						18.0
					],
					"text": "Voice",
					"fontsize": 11.0
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
						600.0,
						120.0,
						16.0
					],
					"presentation": 1,
					"presentation_rect": [
						8.0,
						80.0,
						120.0,
						16.0
					],
					"text": "FohMixer LUFS",
					"fontsize": 9.0
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
						600.0,
						60.0,
						16.0
					],
					"presentation": 1,
					"presentation_rect": [
						140.0,
						80.0,
						60.0,
						16.0
					],
					"text": "v1.60.9",
					"varname": "version",
					"fontsize": 9.0
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
						20.0,
						140.0,
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
						220.0,
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
						260.0,
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
						300.0,
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
						360.0,
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
						400.0,
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
						440.0,
						84.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-parent1-t",
					"maxclass": "newobj",
					"text": "t i i i",
					"numinlets": 1,
					"numoutlets": 3,
					"outlettype": [
						"int",
						"int",
						"int"
					],
					"patching_rect": [
						420.0,
						480.0,
						60.0,
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
						520.0,
						60.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-parent1-observe",
					"maxclass": "newobj",
					"text": "prepend id",
					"numinlets": 1,
					"numoutlets": 1,
					"outlettype": [
						""
					],
					"patching_rect": [
						490.0,
						520.0,
						80.0,
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
						580.0,
						520.0,
						80.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-parent1-empty",
					"maxclass": "newobj",
					"text": "t b b",
					"numinlets": 1,
					"numoutlets": 2,
					"outlettype": [
						"bang",
						"bang"
					],
					"patching_rect": [
						420.0,
						560.0,
						50.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-parent1-observer",
					"maxclass": "newobj",
					"text": "live.observer",
					"numinlets": 2,
					"numoutlets": 2,
					"outlettype": [
						"",
						""
					],
					"patching_rect": [
						490.0,
						560.0,
						92.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-parent1-changed",
					"maxclass": "newobj",
					"text": "t b",
					"numinlets": 1,
					"numoutlets": 1,
					"outlettype": [
						"bang"
					],
					"patching_rect": [
						490.0,
						600.0,
						30.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-parent1-read",
					"maxclass": "newobj",
					"text": "t b b b",
					"numinlets": 1,
					"numoutlets": 3,
					"outlettype": [
						"bang",
						"bang",
						"bang"
					],
					"patching_rect": [
						420.0,
						640.0,
						60.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-parent1-none",
					"maxclass": "message",
					"text": "-",
					"numinlets": 2,
					"numoutlets": 1,
					"outlettype": [
						""
					],
					"patching_rect": [
						420.0,
						680.0,
						28.0,
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
						490.0,
						680.0,
						70.0,
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
						490.0,
						720.0,
						80.0,
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
						490.0,
						760.0,
						80.0,
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
						800.0,
						50.0,
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
						840.0,
						60.0,
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
						360.0,
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
						400.0,
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
						440.0,
						84.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-parent2-t",
					"maxclass": "newobj",
					"text": "t i i i",
					"numinlets": 1,
					"numoutlets": 3,
					"outlettype": [
						"int",
						"int",
						"int"
					],
					"patching_rect": [
						680.0,
						480.0,
						60.0,
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
						520.0,
						60.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-parent2-observe",
					"maxclass": "newobj",
					"text": "prepend id",
					"numinlets": 1,
					"numoutlets": 1,
					"outlettype": [
						""
					],
					"patching_rect": [
						750.0,
						520.0,
						80.0,
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
						840.0,
						520.0,
						80.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-parent2-empty",
					"maxclass": "newobj",
					"text": "t b b",
					"numinlets": 1,
					"numoutlets": 2,
					"outlettype": [
						"bang",
						"bang"
					],
					"patching_rect": [
						680.0,
						560.0,
						50.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-parent2-observer",
					"maxclass": "newobj",
					"text": "live.observer",
					"numinlets": 2,
					"numoutlets": 2,
					"outlettype": [
						"",
						""
					],
					"patching_rect": [
						750.0,
						560.0,
						92.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-parent2-changed",
					"maxclass": "newobj",
					"text": "t b",
					"numinlets": 1,
					"numoutlets": 1,
					"outlettype": [
						"bang"
					],
					"patching_rect": [
						750.0,
						600.0,
						30.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-parent2-read",
					"maxclass": "newobj",
					"text": "t b b b",
					"numinlets": 1,
					"numoutlets": 3,
					"outlettype": [
						"bang",
						"bang",
						"bang"
					],
					"patching_rect": [
						680.0,
						640.0,
						60.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-parent2-none",
					"maxclass": "message",
					"text": "-",
					"numinlets": 2,
					"numoutlets": 1,
					"outlettype": [
						""
					],
					"patching_rect": [
						680.0,
						680.0,
						28.0,
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
						750.0,
						680.0,
						70.0,
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
						750.0,
						720.0,
						80.0,
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
						750.0,
						760.0,
						80.0,
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
						800.0,
						50.0,
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
						840.0,
						60.0,
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
						360.0,
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
						400.0,
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
						440.0,
						84.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-parent3-t",
					"maxclass": "newobj",
					"text": "t i i i",
					"numinlets": 1,
					"numoutlets": 3,
					"outlettype": [
						"int",
						"int",
						"int"
					],
					"patching_rect": [
						940.0,
						480.0,
						60.0,
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
						520.0,
						60.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-parent3-observe",
					"maxclass": "newobj",
					"text": "prepend id",
					"numinlets": 1,
					"numoutlets": 1,
					"outlettype": [
						""
					],
					"patching_rect": [
						1010.0,
						520.0,
						80.0,
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
						1100.0,
						520.0,
						80.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-parent3-empty",
					"maxclass": "newobj",
					"text": "t b b",
					"numinlets": 1,
					"numoutlets": 2,
					"outlettype": [
						"bang",
						"bang"
					],
					"patching_rect": [
						940.0,
						560.0,
						50.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-parent3-observer",
					"maxclass": "newobj",
					"text": "live.observer",
					"numinlets": 2,
					"numoutlets": 2,
					"outlettype": [
						"",
						""
					],
					"patching_rect": [
						1010.0,
						560.0,
						92.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-parent3-changed",
					"maxclass": "newobj",
					"text": "t b",
					"numinlets": 1,
					"numoutlets": 1,
					"outlettype": [
						"bang"
					],
					"patching_rect": [
						1010.0,
						600.0,
						30.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-parent3-read",
					"maxclass": "newobj",
					"text": "t b b b",
					"numinlets": 1,
					"numoutlets": 3,
					"outlettype": [
						"bang",
						"bang",
						"bang"
					],
					"patching_rect": [
						940.0,
						640.0,
						60.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-parent3-none",
					"maxclass": "message",
					"text": "-",
					"numinlets": 2,
					"numoutlets": 1,
					"outlettype": [
						""
					],
					"patching_rect": [
						940.0,
						680.0,
						28.0,
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
						1010.0,
						680.0,
						70.0,
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
						1010.0,
						720.0,
						80.0,
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
						1010.0,
						760.0,
						80.0,
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
						800.0,
						50.0,
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
						840.0,
						60.0,
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
						360.0,
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
						400.0,
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
						440.0,
						84.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-parent4-t",
					"maxclass": "newobj",
					"text": "t i i",
					"numinlets": 1,
					"numoutlets": 2,
					"outlettype": [
						"int",
						"int"
					],
					"patching_rect": [
						1200.0,
						480.0,
						60.0,
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
						520.0,
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
						1360.0,
						520.0,
						80.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-parent4-empty",
					"maxclass": "newobj",
					"text": "t b b",
					"numinlets": 1,
					"numoutlets": 2,
					"outlettype": [
						"bang",
						"bang"
					],
					"patching_rect": [
						1200.0,
						560.0,
						50.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-parent4-read",
					"maxclass": "newobj",
					"text": "t b b b",
					"numinlets": 1,
					"numoutlets": 3,
					"outlettype": [
						"bang",
						"bang",
						"bang"
					],
					"patching_rect": [
						1200.0,
						640.0,
						60.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-parent4-none",
					"maxclass": "message",
					"text": "-",
					"numinlets": 2,
					"numoutlets": 1,
					"outlettype": [
						""
					],
					"patching_rect": [
						1200.0,
						680.0,
						28.0,
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
						1270.0,
						680.0,
						70.0,
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
						1270.0,
						720.0,
						80.0,
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
						1270.0,
						760.0,
						80.0,
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
						800.0,
						50.0,
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
						840.0,
						60.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-send-trigger",
					"maxclass": "newobj",
					"text": "t b f",
					"numinlets": 1,
					"numoutlets": 2,
					"outlettype": [
						"bang",
						"float"
					],
					"patching_rect": [
						30.0,
						420.0,
						92.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-id-load",
					"maxclass": "newobj",
					"text": "t b b b b b b b b b b",
					"numinlets": 1,
					"numoutlets": 10,
					"outlettype": [
						"bang",
						"bang",
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
						140.0,
						200.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-id-property",
					"maxclass": "message",
					"text": "property name",
					"numinlets": 2,
					"numoutlets": 1,
					"outlettype": [
						""
					],
					"patching_rect": [
						820.0,
						180.0,
						92.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-id-offset",
					"maxclass": "newobj",
					"text": "random 50",
					"numinlets": 2,
					"numoutlets": 1,
					"outlettype": [
						"int"
					],
					"patching_rect": [
						1200.0,
						60.0,
						90.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-id-tick",
					"maxclass": "newobj",
					"text": "t b b b b",
					"numinlets": 1,
					"numoutlets": 4,
					"outlettype": [
						"bang",
						"bang",
						"bang",
						"bang"
					],
					"patching_rect": [
						1200.0,
						260.0,
						120.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-pull-gate",
					"maxclass": "newobj",
					"text": "gate",
					"numinlets": 2,
					"numoutlets": 1,
					"outlettype": [
						""
					],
					"patching_rect": [
						130.0,
						100.0,
						50.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-pull",
					"maxclass": "newobj",
					"text": "t b b b b",
					"numinlets": 1,
					"numoutlets": 4,
					"outlettype": [
						"bang",
						"bang",
						"bang",
						"bang"
					],
					"patching_rect": [
						130.0,
						130.0,
						80.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-poll-receive",
					"maxclass": "newobj",
					"text": "udpreceive 39999 @defer 1",
					"numinlets": 1,
					"numoutlets": 1,
					"outlettype": [
						""
					],
					"patching_rect": [
						1500.0,
						100.0,
						160.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-poll-route",
					"maxclass": "newobj",
					"text": "route /fohmixer/lufs/poll",
					"numinlets": 2,
					"numoutlets": 2,
					"outlettype": [
						"",
						""
					],
					"patching_rect": [
						1500.0,
						140.0,
						191.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-poll-draw",
					"maxclass": "newobj",
					"text": "random 10000",
					"numinlets": 2,
					"numoutlets": 1,
					"outlettype": [
						""
					],
					"patching_rect": [
						1500.0,
						20.0,
						100.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-poll-offset",
					"maxclass": "newobj",
					"text": "+ 40000",
					"numinlets": 2,
					"numoutlets": 1,
					"outlettype": [
						"int"
					],
					"patching_rect": [
						1500.0,
						50.0,
						65.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-poll-port",
					"maxclass": "newobj",
					"text": "t i i",
					"numinlets": 1,
					"numoutlets": 2,
					"outlettype": [
						"int",
						"int"
					],
					"patching_rect": [
						1620.0,
						50.0,
						51.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-poll-set",
					"maxclass": "newobj",
					"text": "prepend port",
					"numinlets": 1,
					"numoutlets": 1,
					"outlettype": [
						""
					],
					"patching_rect": [
						1620.0,
						80.0,
						100.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-active-property",
					"maxclass": "message",
					"text": "property is_active",
					"numinlets": 2,
					"numoutlets": 1,
					"outlettype": [
						""
					],
					"patching_rect": [
						1500.0,
						200.0,
						142.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-active-path",
					"maxclass": "message",
					"text": "path this_device",
					"numinlets": 2,
					"numoutlets": 1,
					"outlettype": [
						""
					],
					"patching_rect": [
						1680.0,
						200.0,
						128.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-active-live-path",
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
						1680.0,
						240.0,
						79.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-active-observer",
					"maxclass": "newobj",
					"text": "live.observer",
					"numinlets": 2,
					"numoutlets": 2,
					"outlettype": [
						"",
						""
					],
					"patching_rect": [
						1500.0,
						280.0,
						107.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-active-t",
					"maxclass": "newobj",
					"text": "t i i",
					"numinlets": 1,
					"numoutlets": 2,
					"outlettype": [
						"int",
						"int"
					],
					"patching_rect": [
						1500.0,
						320.0,
						51.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-active-sel",
					"maxclass": "newobj",
					"text": "sel 1",
					"numinlets": 2,
					"numoutlets": 2,
					"outlettype": [
						"bang",
						""
					],
					"patching_rect": [
						1500.0,
						360.0,
						51.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-sr-state",
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
						1800.0,
						20.0,
						80.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-sr",
					"maxclass": "newobj",
					"text": "t f f f f",
					"numinlets": 1,
					"numoutlets": 4,
					"outlettype": [
						"float",
						"float",
						"float",
						"float"
					],
					"patching_rect": [
						1800.0,
						60.0,
						80.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-shelf-k",
					"maxclass": "newobj",
					"text": "expr tan(3.141592653589793*1681.974450955533/$f1)",
					"numinlets": 1,
					"numoutlets": 1,
					"outlettype": [
						""
					],
					"patching_rect": [
						1800.0,
						100.0,
						320.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-shelf-t",
					"maxclass": "newobj",
					"text": "t f f f f f",
					"numinlets": 1,
					"numoutlets": 5,
					"outlettype": [
						"float",
						"float",
						"float",
						"float",
						"float"
					],
					"patching_rect": [
						1800.0,
						140.0,
						100.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-shelf-b0",
					"maxclass": "newobj",
					"text": "expr (pow(10.\\, 3.999843853973347/20.)+pow(pow(10.\\, 3.999843853973347/20.)\\, 0.4996667741545416)*$f1/0.7071752369554196+$f1*$f1)/(1.+$f1/0.7071752369554196+$f1*$f1)",
					"numinlets": 1,
					"numoutlets": 1,
					"outlettype": [
						""
					],
					"patching_rect": [
						1800.0,
						180.0,
						900.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-shelf-b1",
					"maxclass": "newobj",
					"text": "expr 2.*($f1*$f1-pow(10.\\, 3.999843853973347/20.))/(1.+$f1/0.7071752369554196+$f1*$f1)",
					"numinlets": 1,
					"numoutlets": 1,
					"outlettype": [
						""
					],
					"patching_rect": [
						1800.0,
						220.0,
						900.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-shelf-b2",
					"maxclass": "newobj",
					"text": "expr (pow(10.\\, 3.999843853973347/20.)-pow(pow(10.\\, 3.999843853973347/20.)\\, 0.4996667741545416)*$f1/0.7071752369554196+$f1*$f1)/(1.+$f1/0.7071752369554196+$f1*$f1)",
					"numinlets": 1,
					"numoutlets": 1,
					"outlettype": [
						""
					],
					"patching_rect": [
						1800.0,
						260.0,
						900.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-shelf-a1",
					"maxclass": "newobj",
					"text": "expr 2.*($f1*$f1-1.)/(1.+$f1/0.7071752369554196+$f1*$f1)",
					"numinlets": 1,
					"numoutlets": 1,
					"outlettype": [
						""
					],
					"patching_rect": [
						1800.0,
						300.0,
						900.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-shelf-a2",
					"maxclass": "newobj",
					"text": "expr (1.-$f1/0.7071752369554196+$f1*$f1)/(1.+$f1/0.7071752369554196+$f1*$f1)",
					"numinlets": 1,
					"numoutlets": 1,
					"outlettype": [
						""
					],
					"patching_rect": [
						1800.0,
						340.0,
						900.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-shelf-pack",
					"maxclass": "newobj",
					"text": "pack 0. 0. 0. 0. 0.",
					"numinlets": 5,
					"numoutlets": 1,
					"outlettype": [
						""
					],
					"patching_rect": [
						1800.0,
						380.0,
						160.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-hp-k",
					"maxclass": "newobj",
					"text": "expr tan(3.141592653589793*38.13547087602444/$f1)",
					"numinlets": 1,
					"numoutlets": 1,
					"outlettype": [
						""
					],
					"patching_rect": [
						1800.0,
						440.0,
						320.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-hp-t",
					"maxclass": "newobj",
					"text": "t b f f",
					"numinlets": 1,
					"numoutlets": 3,
					"outlettype": [
						"bang",
						"float",
						"float"
					],
					"patching_rect": [
						1800.0,
						480.0,
						80.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-hp-a1",
					"maxclass": "newobj",
					"text": "expr 2.*($f1*$f1-1.)/(1.+$f1/0.5003270373238773+$f1*$f1)",
					"numinlets": 1,
					"numoutlets": 1,
					"outlettype": [
						""
					],
					"patching_rect": [
						1800.0,
						520.0,
						520.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-hp-a2",
					"maxclass": "newobj",
					"text": "expr (1.-$f1/0.5003270373238773+$f1*$f1)/(1.+$f1/0.5003270373238773+$f1*$f1)",
					"numinlets": 1,
					"numoutlets": 1,
					"outlettype": [
						""
					],
					"patching_rect": [
						1800.0,
						560.0,
						520.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-hp-pack",
					"maxclass": "newobj",
					"text": "pack 1. -2. 1. 0. 0.",
					"numinlets": 5,
					"numoutlets": 1,
					"outlettype": [
						""
					],
					"patching_rect": [
						1800.0,
						600.0,
						160.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-window-short",
					"maxclass": "newobj",
					"text": "expr int(3.0*$f1+0.5)",
					"numinlets": 1,
					"numoutlets": 1,
					"outlettype": [
						""
					],
					"patching_rect": [
						1800.0,
						660.0,
						180.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-window-momentary",
					"maxclass": "newobj",
					"text": "expr int(0.4*$f1+0.5)",
					"numinlets": 1,
					"numoutlets": 1,
					"outlettype": [
						""
					],
					"patching_rect": [
						2000.0,
						660.0,
						180.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-shelf-l",
					"maxclass": "newobj",
					"text": "biquad~",
					"numinlets": 6,
					"numoutlets": 1,
					"outlettype": [
						"signal"
					],
					"patching_rect": [
						30.0,
						170.0,
						80.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-shelf-r",
					"maxclass": "newobj",
					"text": "biquad~",
					"numinlets": 6,
					"numoutlets": 1,
					"outlettype": [
						"signal"
					],
					"patching_rect": [
						130.0,
						170.0,
						80.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-hp-l",
					"maxclass": "newobj",
					"text": "biquad~",
					"numinlets": 6,
					"numoutlets": 1,
					"outlettype": [
						"signal"
					],
					"patching_rect": [
						30.0,
						200.0,
						80.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-hp-r",
					"maxclass": "newobj",
					"text": "biquad~",
					"numinlets": 6,
					"numoutlets": 1,
					"outlettype": [
						"signal"
					],
					"patching_rect": [
						130.0,
						200.0,
						80.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-square-l",
					"maxclass": "newobj",
					"text": "*~",
					"numinlets": 2,
					"numoutlets": 1,
					"outlettype": [
						"signal"
					],
					"patching_rect": [
						30.0,
						230.0,
						40.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-square-r",
					"maxclass": "newobj",
					"text": "*~",
					"numinlets": 2,
					"numoutlets": 1,
					"outlettype": [
						"signal"
					],
					"patching_rect": [
						130.0,
						230.0,
						40.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-sum",
					"maxclass": "newobj",
					"text": "+~",
					"numinlets": 2,
					"numoutlets": 1,
					"outlettype": [
						"signal"
					],
					"patching_rect": [
						30.0,
						260.0,
						40.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-avg-short",
					"maxclass": "newobj",
					"text": "average~ 576000 bipolar",
					"numinlets": 1,
					"numoutlets": 1,
					"outlettype": [
						"signal"
					],
					"patching_rect": [
						30.0,
						300.0,
						150.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-avg-momentary",
					"maxclass": "newobj",
					"text": "average~ 76800 bipolar",
					"numinlets": 1,
					"numoutlets": 1,
					"outlettype": [
						"signal"
					],
					"patching_rect": [
						200.0,
						300.0,
						150.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-gate",
					"maxclass": "newobj",
					"text": ">=~ 0.00011724653045822968",
					"numinlets": 2,
					"numoutlets": 1,
					"outlettype": [
						"signal"
					],
					"patching_rect": [
						200.0,
						330.0,
						180.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-voiced",
					"maxclass": "newobj",
					"text": "*~",
					"numinlets": 2,
					"numoutlets": 1,
					"outlettype": [
						"signal"
					],
					"patching_rect": [
						380.0,
						360.0,
						40.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-avg-share",
					"maxclass": "newobj",
					"text": "average~ 576000 bipolar",
					"numinlets": 1,
					"numoutlets": 1,
					"outlettype": [
						"signal"
					],
					"patching_rect": [
						200.0,
						390.0,
						150.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-avg-voiced",
					"maxclass": "newobj",
					"text": "average~ 576000 bipolar",
					"numinlets": 1,
					"numoutlets": 1,
					"outlettype": [
						"signal"
					],
					"patching_rect": [
						380.0,
						390.0,
						150.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-snap-g",
					"maxclass": "newobj",
					"text": "snapshot~ 0",
					"numinlets": 2,
					"numoutlets": 1,
					"outlettype": [
						"float"
					],
					"patching_rect": [
						200.0,
						420.0,
						80.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-level-s",
					"maxclass": "newobj",
					"text": "expr 10.*log10(max($f1\\, pow(0.1\\, 30.)))-0.691",
					"numinlets": 1,
					"numoutlets": 1,
					"outlettype": [
						""
					],
					"patching_rect": [
						130.0,
						450.0,
						300.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-level-v",
					"maxclass": "newobj",
					"text": "expr ($f2>=10.0/30.0)*(10.*log10(max($f1/max($f2\\, pow(0.1\\, 30.))\\, pow(0.1\\, 30.)))-0.691)-70.0*($f2<10.0/30.0)",
					"numinlets": 2,
					"numoutlets": 1,
					"outlettype": [
						""
					],
					"patching_rect": [
						440.0,
						450.0,
						700.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-face-s",
					"maxclass": "newobj",
					"text": "* 0.1",
					"numinlets": 2,
					"numoutlets": 1,
					"outlettype": [
						""
					],
					"patching_rect": [
						130.0,
						560.0,
						50.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-face-v",
					"maxclass": "newobj",
					"text": "* 0.1",
					"numinlets": 2,
					"numoutlets": 1,
					"outlettype": [
						""
					],
					"patching_rect": [
						440.0,
						560.0,
						50.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-id-offset-set",
					"maxclass": "newobj",
					"text": "prepend set",
					"numinlets": 1,
					"numoutlets": 1,
					"outlettype": [
						""
					],
					"patching_rect": [
						1200.0,
						140.0,
						80.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-id-count",
					"maxclass": "newobj",
					"text": "counter 0 49",
					"numinlets": 4,
					"numoutlets": 4,
					"outlettype": [
						"int",
						"",
						"",
						"int"
					],
					"patching_rect": [
						1200.0,
						180.0,
						90.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-id-every",
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
						220.0,
						50.0,
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
						"obj-times-s",
						0
					],
					"destination": [
						"obj-round-s",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-round-s",
						0
					],
					"destination": [
						"obj-clip-s",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-clip-s",
						0
					],
					"destination": [
						"obj-pack",
						8
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-times-v",
						0
					],
					"destination": [
						"obj-round-v",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-round-v",
						0
					],
					"destination": [
						"obj-clip-v",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-clip-v",
						0
					],
					"destination": [
						"obj-send-trigger",
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
						"obj-id-route",
						0
					],
					"destination": [
						"obj-pack",
						2
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
						"obj-send-trigger",
						1
					],
					"destination": [
						"obj-pack",
						9
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-send-trigger",
						0
					],
					"destination": [
						"obj-pack",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-id-load",
						8
					],
					"destination": [
						"obj-id-property",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-id-load",
						7
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
						"obj-id-load",
						6
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
						"obj-parent1-live-path",
						1
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
						"obj-id-load",
						5
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
						"obj-parent2-live-path",
						1
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
						"obj-id-load",
						4
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
						"obj-parent3-live-path",
						1
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
						"obj-id-load",
						3
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
						"obj-parent4-live-path",
						1
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
						"obj-parent1-route-id",
						0
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
						2
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
						"obj-parent1-sel",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent1-sel",
						0
					],
					"destination": [
						"obj-parent1-empty",
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
						"obj-parent1-read",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent1-changed",
						0
					],
					"destination": [
						"obj-parent1-read",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent1-read",
						2
					],
					"destination": [
						"obj-parent1-none",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent1-empty",
						1
					],
					"destination": [
						"obj-parent1-none",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent1-read",
						1
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
						"obj-parent1-read",
						0
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
						"obj-parent1-empty",
						0
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
						"obj-parent1-none",
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
						"obj-parent1-symbol",
						0
					],
					"destination": [
						"obj-pack",
						4
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
						"obj-parent1-observe",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-id-property",
						0
					],
					"destination": [
						"obj-parent1-observer",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent1-observe",
						0
					],
					"destination": [
						"obj-parent1-observer",
						1
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent1-observer",
						0
					],
					"destination": [
						"obj-parent1-changed",
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
						"obj-parent2-t",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent2-t",
						2
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
						"obj-parent2-sel",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent2-sel",
						0
					],
					"destination": [
						"obj-parent2-empty",
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
						"obj-parent2-read",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent2-changed",
						0
					],
					"destination": [
						"obj-parent2-read",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent2-read",
						2
					],
					"destination": [
						"obj-parent2-none",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent2-empty",
						1
					],
					"destination": [
						"obj-parent2-none",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent2-read",
						1
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
						"obj-parent2-read",
						0
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
						"obj-parent2-empty",
						0
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
						"obj-parent2-none",
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
						"obj-parent2-symbol",
						0
					],
					"destination": [
						"obj-pack",
						5
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
						"obj-parent2-observe",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-id-property",
						0
					],
					"destination": [
						"obj-parent2-observer",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent2-observe",
						0
					],
					"destination": [
						"obj-parent2-observer",
						1
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent2-observer",
						0
					],
					"destination": [
						"obj-parent2-changed",
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
						"obj-parent3-t",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent3-t",
						2
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
						"obj-parent3-sel",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent3-sel",
						0
					],
					"destination": [
						"obj-parent3-empty",
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
						"obj-parent3-read",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent3-changed",
						0
					],
					"destination": [
						"obj-parent3-read",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent3-read",
						2
					],
					"destination": [
						"obj-parent3-none",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent3-empty",
						1
					],
					"destination": [
						"obj-parent3-none",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent3-read",
						1
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
						"obj-parent3-read",
						0
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
						"obj-parent3-empty",
						0
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
						"obj-parent3-none",
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
						"obj-parent3-symbol",
						0
					],
					"destination": [
						"obj-pack",
						6
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
						"obj-parent3-observe",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-id-property",
						0
					],
					"destination": [
						"obj-parent3-observer",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent3-observe",
						0
					],
					"destination": [
						"obj-parent3-observer",
						1
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent3-observer",
						0
					],
					"destination": [
						"obj-parent3-changed",
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
						"obj-parent4-sel",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent4-sel",
						0
					],
					"destination": [
						"obj-parent4-empty",
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
						"obj-parent4-read",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent4-read",
						2
					],
					"destination": [
						"obj-parent4-none",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent4-empty",
						1
					],
					"destination": [
						"obj-parent4-none",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-parent4-read",
						1
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
						"obj-parent4-read",
						0
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
						"obj-parent4-empty",
						0
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
						"obj-parent4-none",
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
						"obj-parent4-symbol",
						0
					],
					"destination": [
						"obj-pack",
						7
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
						"obj-id-offset",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-id-tick",
						3
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
						"obj-id-tick",
						2
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
						"obj-id-tick",
						1
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
						"obj-id-tick",
						0
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
						"obj-pack",
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
						"obj-pull-gate",
						0
					],
					"destination": [
						"obj-pull",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-poll-receive",
						0
					],
					"destination": [
						"obj-poll-route",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-poll-route",
						0
					],
					"destination": [
						"obj-pull-gate",
						1
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-id-load",
						9
					],
					"destination": [
						"obj-poll-draw",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-poll-draw",
						0
					],
					"destination": [
						"obj-poll-offset",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-poll-offset",
						0
					],
					"destination": [
						"obj-poll-port",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-poll-port",
						1
					],
					"destination": [
						"obj-poll-set",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-poll-set",
						0
					],
					"destination": [
						"obj-poll-receive",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-poll-port",
						0
					],
					"destination": [
						"obj-pack",
						10
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-id-load",
						2
					],
					"destination": [
						"obj-active-property",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-id-load",
						1
					],
					"destination": [
						"obj-active-path",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-active-property",
						0
					],
					"destination": [
						"obj-active-observer",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-active-path",
						0
					],
					"destination": [
						"obj-active-live-path",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-active-live-path",
						0
					],
					"destination": [
						"obj-active-observer",
						1
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-active-observer",
						0
					],
					"destination": [
						"obj-active-t",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-active-t",
						1
					],
					"destination": [
						"obj-pull-gate",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-active-t",
						0
					],
					"destination": [
						"obj-active-sel",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-active-sel",
						0
					],
					"destination": [
						"obj-pull-gate",
						1
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-id-load",
						0
					],
					"destination": [
						"obj-pull-gate",
						1
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-sr-state",
						1
					],
					"destination": [
						"obj-sr",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-sr",
						3
					],
					"destination": [
						"obj-shelf-k",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-sr",
						2
					],
					"destination": [
						"obj-hp-k",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-sr",
						1
					],
					"destination": [
						"obj-window-short",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-sr",
						0
					],
					"destination": [
						"obj-window-momentary",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-shelf-k",
						0
					],
					"destination": [
						"obj-shelf-t",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-hp-k",
						0
					],
					"destination": [
						"obj-hp-t",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-hp-t",
						2
					],
					"destination": [
						"obj-hp-a2",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-hp-t",
						1
					],
					"destination": [
						"obj-hp-a1",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-hp-t",
						0
					],
					"destination": [
						"obj-hp-pack",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-hp-a1",
						0
					],
					"destination": [
						"obj-hp-pack",
						3
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-hp-a2",
						0
					],
					"destination": [
						"obj-hp-pack",
						4
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
						"obj-shelf-l",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-shelf-pack",
						0
					],
					"destination": [
						"obj-shelf-l",
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
						"obj-shelf-r",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-shelf-pack",
						0
					],
					"destination": [
						"obj-shelf-r",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-shelf-l",
						0
					],
					"destination": [
						"obj-hp-l",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-hp-pack",
						0
					],
					"destination": [
						"obj-hp-l",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-shelf-r",
						0
					],
					"destination": [
						"obj-hp-r",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-hp-pack",
						0
					],
					"destination": [
						"obj-hp-r",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-hp-l",
						0
					],
					"destination": [
						"obj-square-l",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-hp-l",
						0
					],
					"destination": [
						"obj-square-l",
						1
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-hp-r",
						0
					],
					"destination": [
						"obj-square-r",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-hp-r",
						0
					],
					"destination": [
						"obj-square-r",
						1
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-square-l",
						0
					],
					"destination": [
						"obj-sum",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-square-r",
						0
					],
					"destination": [
						"obj-sum",
						1
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-sum",
						0
					],
					"destination": [
						"obj-avg-short",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-window-short",
						0
					],
					"destination": [
						"obj-avg-short",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-sum",
						0
					],
					"destination": [
						"obj-avg-momentary",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-window-momentary",
						0
					],
					"destination": [
						"obj-avg-momentary",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-avg-momentary",
						0
					],
					"destination": [
						"obj-gate",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-gate",
						0
					],
					"destination": [
						"obj-avg-share",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-window-short",
						0
					],
					"destination": [
						"obj-avg-share",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-sum",
						0
					],
					"destination": [
						"obj-voiced",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-gate",
						0
					],
					"destination": [
						"obj-voiced",
						1
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-voiced",
						0
					],
					"destination": [
						"obj-avg-voiced",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-window-short",
						0
					],
					"destination": [
						"obj-avg-voiced",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-avg-short",
						0
					],
					"destination": [
						"obj-snap-s",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-pull",
						2
					],
					"destination": [
						"obj-snap-s",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-avg-share",
						0
					],
					"destination": [
						"obj-snap-g",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-pull",
						1
					],
					"destination": [
						"obj-snap-g",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-avg-voiced",
						0
					],
					"destination": [
						"obj-snap-v",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-pull",
						0
					],
					"destination": [
						"obj-snap-v",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-snap-s",
						0
					],
					"destination": [
						"obj-level-s",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-snap-v",
						0
					],
					"destination": [
						"obj-level-v",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-snap-g",
						0
					],
					"destination": [
						"obj-level-v",
						1
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-level-s",
						0
					],
					"destination": [
						"obj-times-s",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-level-v",
						0
					],
					"destination": [
						"obj-times-v",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-clip-s",
						0
					],
					"destination": [
						"obj-face-s",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-clip-v",
						0
					],
					"destination": [
						"obj-face-v",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-face-s",
						0
					],
					"destination": [
						"obj-short",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-face-v",
						0
					],
					"destination": [
						"obj-voice",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-shelf-b0",
						0
					],
					"destination": [
						"obj-shelf-pack",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-shelf-b1",
						0
					],
					"destination": [
						"obj-shelf-pack",
						1
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-shelf-b2",
						0
					],
					"destination": [
						"obj-shelf-pack",
						2
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-shelf-a1",
						0
					],
					"destination": [
						"obj-shelf-pack",
						3
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-shelf-a2",
						0
					],
					"destination": [
						"obj-shelf-pack",
						4
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-shelf-t",
						0
					],
					"destination": [
						"obj-shelf-b0",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-shelf-t",
						1
					],
					"destination": [
						"obj-shelf-b1",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-shelf-t",
						2
					],
					"destination": [
						"obj-shelf-b2",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-shelf-t",
						3
					],
					"destination": [
						"obj-shelf-a1",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-shelf-t",
						4
					],
					"destination": [
						"obj-shelf-a2",
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
						"obj-id-load",
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
						"obj-sr-state",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-id-offset",
						0
					],
					"destination": [
						"obj-id-offset-set",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-id-offset-set",
						0
					],
					"destination": [
						"obj-id-count",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-pull",
						3
					],
					"destination": [
						"obj-id-count",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-id-count",
						0
					],
					"destination": [
						"obj-id-every",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-id-every",
						0
					],
					"destination": [
						"obj-id-tick",
						0
					]
				}
			}
		],
		"dependency_cache": [],
		"autosave": 0
	}
}
