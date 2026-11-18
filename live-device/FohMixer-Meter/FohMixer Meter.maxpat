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
		"description": "FohMixer Meter: the L and R peak levels, sent to the FohMixer hub on 127.0.0.1.",
		"devicewidth": 180.0,
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
					"id": "obj-pack",
					"maxclass": "newobj",
					"text": "pack /fohmixer/meter 1 0 v1.54.4 - - - - -700 -700",
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
					"id": "obj-peak-l",
					"maxclass": "newobj",
					"text": "peakamp~ 33",
					"numinlets": 2,
					"numoutlets": 1,
					"outlettype": [
						"float"
					],
					"patching_rect": [
						130.0,
						120.0,
						108.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-db-l",
					"maxclass": "newobj",
					"text": "atodb",
					"numinlets": 1,
					"numoutlets": 1,
					"outlettype": [
						""
					],
					"patching_rect": [
						130.0,
						160.0,
						60.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-show-l",
					"maxclass": "newobj",
					"text": "clip -70. 12.",
					"numinlets": 3,
					"numoutlets": 1,
					"outlettype": [
						""
					],
					"patching_rect": [
						130.0,
						200.0,
						124.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-times-l",
					"maxclass": "newobj",
					"text": "* 10.",
					"numinlets": 2,
					"numoutlets": 1,
					"outlettype": [
						""
					],
					"patching_rect": [
						130.0,
						240.0,
						60.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-round-l",
					"maxclass": "newobj",
					"text": "round 1.",
					"numinlets": 2,
					"numoutlets": 1,
					"outlettype": [
						""
					],
					"patching_rect": [
						130.0,
						280.0,
						84.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-clip-l",
					"maxclass": "newobj",
					"text": "clip -700 120",
					"numinlets": 3,
					"numoutlets": 1,
					"outlettype": [
						""
					],
					"patching_rect": [
						130.0,
						320.0,
						124.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-peak-r",
					"maxclass": "newobj",
					"text": "peakamp~ 33",
					"numinlets": 2,
					"numoutlets": 1,
					"outlettype": [
						"float"
					],
					"patching_rect": [
						240.0,
						120.0,
						108.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-db-r",
					"maxclass": "newobj",
					"text": "atodb",
					"numinlets": 1,
					"numoutlets": 1,
					"outlettype": [
						""
					],
					"patching_rect": [
						240.0,
						160.0,
						60.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-show-r",
					"maxclass": "newobj",
					"text": "clip -70. 12.",
					"numinlets": 3,
					"numoutlets": 1,
					"outlettype": [
						""
					],
					"patching_rect": [
						240.0,
						200.0,
						124.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-times-r",
					"maxclass": "newobj",
					"text": "* 10.",
					"numinlets": 2,
					"numoutlets": 1,
					"outlettype": [
						""
					],
					"patching_rect": [
						240.0,
						240.0,
						60.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-round-r",
					"maxclass": "newobj",
					"text": "round 1.",
					"numinlets": 2,
					"numoutlets": 1,
					"outlettype": [
						""
					],
					"patching_rect": [
						240.0,
						280.0,
						84.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-clip-r",
					"maxclass": "newobj",
					"text": "clip -700 120",
					"numinlets": 3,
					"numoutlets": 1,
					"outlettype": [
						""
					],
					"patching_rect": [
						240.0,
						320.0,
						124.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-left",
					"maxclass": "flonum",
					"numinlets": 1,
					"numoutlets": 2,
					"patching_rect": [
						130.0,
						640.0,
						72.0,
						18.0
					],
					"presentation": 1,
					"presentation_rect": [
						8.0,
						30.0,
						72.0,
						18.0
					],
					"outlettype": [
						"",
						"bang"
					],
					"format": 6,
					"ignoreclick": 1,
					"minimum": -70.0,
					"maximum": 12.0,
					"parameter_enable": 0
				}
			},
			{
				"box": {
					"id": "obj-right",
					"maxclass": "flonum",
					"numinlets": 1,
					"numoutlets": 2,
					"patching_rect": [
						240.0,
						640.0,
						72.0,
						18.0
					],
					"presentation": 1,
					"presentation_rect": [
						88.0,
						30.0,
						72.0,
						18.0
					],
					"outlettype": [
						"",
						"bang"
					],
					"format": 6,
					"ignoreclick": 1,
					"minimum": -70.0,
					"maximum": 12.0,
					"parameter_enable": 0
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
						72.0,
						18.0
					],
					"presentation": 1,
					"presentation_rect": [
						8.0,
						10.0,
						72.0,
						18.0
					],
					"text": "L dBFS"
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
						72.0,
						18.0
					],
					"presentation": 1,
					"presentation_rect": [
						88.0,
						10.0,
						72.0,
						18.0
					],
					"text": "R dBFS"
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
						100.0,
						18.0
					],
					"presentation": 1,
					"presentation_rect": [
						8.0,
						60.0,
						100.0,
						18.0
					],
					"text": "FohMixer Meter"
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
						18.0
					],
					"presentation": 1,
					"presentation_rect": [
						108.0,
						60.0,
						60.0,
						18.0
					],
					"text": "v1.54.4",
					"varname": "version"
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
						100.0,
						84.0,
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
						84.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-send-defer",
					"maxclass": "newobj",
					"text": "deferlow",
					"numinlets": 1,
					"numoutlets": 1,
					"outlettype": [
						""
					],
					"patching_rect": [
						30.0,
						500.0,
						58.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-id-load",
					"maxclass": "newobj",
					"text": "t b b b b b b",
					"numinlets": 1,
					"numoutlets": 6,
					"outlettype": [
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
						140.0,
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
					"text": "random 10000",
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
					"id": "obj-id-offset-delay",
					"maxclass": "newobj",
					"text": "delay",
					"numinlets": 2,
					"numoutlets": 1,
					"outlettype": [
						"bang"
					],
					"patching_rect": [
						1200.0,
						100.0,
						50.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-id-clock-defer",
					"maxclass": "newobj",
					"text": "deferlow",
					"numinlets": 1,
					"numoutlets": 1,
					"outlettype": [
						""
					],
					"patching_rect": [
						1200.0,
						140.0,
						58.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-id-clock-start",
					"maxclass": "message",
					"text": "1",
					"numinlets": 2,
					"numoutlets": 1,
					"outlettype": [
						""
					],
					"patching_rect": [
						1200.0,
						180.0,
						30.0,
						22.0
					]
				}
			},
			{
				"box": {
					"id": "obj-id-clock",
					"maxclass": "newobj",
					"text": "qmetro 10000",
					"numinlets": 2,
					"numoutlets": 1,
					"outlettype": [
						"bang"
					],
					"patching_rect": [
						1200.0,
						220.0,
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
						100.0,
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
						"obj-peak-l",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-peak-l",
						0
					],
					"destination": [
						"obj-db-l",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-db-l",
						0
					],
					"destination": [
						"obj-show-l",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-show-l",
						0
					],
					"destination": [
						"obj-times-l",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-times-l",
						0
					],
					"destination": [
						"obj-round-l",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-round-l",
						0
					],
					"destination": [
						"obj-clip-l",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-clip-l",
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
						"obj-plugin",
						1
					],
					"destination": [
						"obj-peak-r",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-peak-r",
						0
					],
					"destination": [
						"obj-db-r",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-db-r",
						0
					],
					"destination": [
						"obj-show-r",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-show-r",
						0
					],
					"destination": [
						"obj-times-r",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-times-r",
						0
					],
					"destination": [
						"obj-round-r",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-round-r",
						0
					],
					"destination": [
						"obj-clip-r",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-clip-r",
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
						"obj-show-l",
						0
					],
					"destination": [
						"obj-left",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-show-r",
						0
					],
					"destination": [
						"obj-right",
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
						"obj-pack",
						0
					],
					"destination": [
						"obj-send-defer",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-send-defer",
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
						"obj-id-defer",
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
						"obj-id-load",
						5
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
						4
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
						"obj-id-offset",
						0
					],
					"destination": [
						"obj-id-offset-delay",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-id-offset-delay",
						0
					],
					"destination": [
						"obj-id-clock-defer",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-id-clock-defer",
						0
					],
					"destination": [
						"obj-id-clock-start",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-id-clock-start",
						0
					],
					"destination": [
						"obj-id-clock",
						0
					]
				}
			},
			{
				"patchline": {
					"source": [
						"obj-id-clock",
						0
					],
					"destination": [
						"obj-id-tick",
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
			}
		],
		"dependency_cache": [],
		"autosave": 0
	}
}
