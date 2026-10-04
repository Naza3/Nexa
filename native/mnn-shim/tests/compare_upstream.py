#!/usr/bin/env python3
"""Compare generated synthetic reports, not timing or different backends."""
import argparse,json,pathlib
p=argparse.ArgumentParser();p.add_argument('shim',type=pathlib.Path);p.add_argument('unpatched',type=pathlib.Path);a=p.parse_args();ours=json.loads(a.shim.read_text());baseline=json.loads(a.unpatched.read_text())
for key in ['rendered_prompt','prompt_tokens','output_tokens']:assert ours[key]==baseline[key],key
assert ours['shim_text']==baseline['output_text']
assert ours['shim_prompt_count']==len(baseline['prompt_tokens'])
assert ours['shim_completion_count']==len(baseline['output_tokens'])
print(json.dumps({'exact_unpatched_upstream_comparison':'passed','prompt_tokens':ours['shim_prompt_count'],'completion_tokens':ours['shim_completion_count']}))
