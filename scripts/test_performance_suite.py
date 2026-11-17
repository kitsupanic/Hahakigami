import unittest
import json
from pathlib import Path
import tempfile
import sys
from performance_suite import gates, owned_rss, schedule, fingerprints, run, execute
from unittest.mock import patch


class SuiteTests(unittest.TestCase):
    def test_canvas_geometry_requires_real_positive_backing_and_released_empty_maps(self):
        from performance_suite import rendered_canvas_contract
        evidence = dict(expected_positive_nodes=3, backing_width=1600, backing_height=800,
                        css_width=800, css_height=400, device_pixel_ratio=2)
        self.assertTrue(rendered_canvas_contract(evidence))
        self.assertTrue(rendered_canvas_contract(dict(evidence, backing_width=1599)))
        self.assertTrue(rendered_canvas_contract(dict(evidence, expected_positive_nodes=0,
                                                     backing_width=0, backing_height=0)))
        for changed in [None, {}, dict(evidence, backing_width=0),
                        dict(evidence, css_height=0), dict(evidence, backing_width=800),
                        dict(evidence, expected_positive_nodes=True),
                        dict(evidence, expected_positive_nodes=0),
                        dict(evidence, css_width=float('nan')),
                        dict(evidence, device_pixel_ratio=float('inf')),
                        dict(evidence, css_width=1e308, device_pixel_ratio=1e308)]:
            self.assertFalse(rendered_canvas_contract(changed))

    def test_rotating_initial_separate(self):
        rows=list(schedule(['a','b','c'],5))
        self.assertEqual(rows[0],('initial',0,['a','b','c']))
        self.assertEqual(rows[1][2],['b','c','a'])
        self.assertEqual(len(rows),6)

    def test_owned_birth_guard_and_unrelated(self):
        snapshot={10:(1,100,'birth'),11:(10,40,'child'),12:(1,900,'unrelated')}
        self.assertEqual(owned_rss(snapshot,10,'birth',{}),140)
        self.assertEqual(owned_rss(snapshot,10,'reused',{}),0)
        self.assertEqual(owned_rss(snapshot,10,'birth',{11:'old'}),100)

    def measurements(self):
        return [dict(layer=layer,scope=scope,mode='full',phase='warm',role=role,round=n,success=True,coverage_complete=True,requested_mode='full',cache_mode='full',rendered=True,elapsed_ms=100,peak_rss_bytes=100 if role=='baseline' else 75,sampled_peak_aggregate_rss_bytes=100 if role=='baseline' else 75) for layer in ('native','app') for scope in ('home','startup','fixture') for role in ('baseline','candidate') for n in range(1,6)]

    def test_gates_reject_missing_hash_coverage_failure(self):
        runs=self.measurements()
        coverage={f'{layer}/{scope}':{'equivalent':True} for layer in ('native','app') for scope in ('home','startup','fixture')}
        self.assertTrue(gates(runs,coverage,True)['acceptance_met'])
        self.assertFalse(gates(runs,coverage,False)['acceptance_met'])
        self.assertFalse(gates(runs,{},True)['acceptance_met'])
        runs[0]['success']=False
        self.assertEqual(gates(runs,coverage,True)['gates']['native/home/full']['status'],'inconclusive')

    def test_regression_fails(self):
        runs=self.measurements()
        for r in runs:
            if r['role']=='candidate': r['elapsed_ms']=106
        coverage={f'{layer}/{scope}':{'equivalent':True} for layer in ('native','app') for scope in ('home','startup')}
        self.assertEqual(gates(runs,coverage,True)['gates']['native/home/full']['status'],'fail')

    def test_hash_changes_and_malformed_child(self):
        with tempfile.TemporaryDirectory() as d:
            source = Path(d)/'source'
            source.write_bytes(b'before')
            executable = {'id':'test','path':sys.executable,'role':'candidate','layer':'native','source_paths':[str(source)],'protocol':'adapter','args':['-c','print("malformed")']}
            before = fingerprints([executable])
            source.write_bytes(b'after')
            self.assertNotEqual(before,fingerprints([executable]))
            record = run(executable,{},5)
            self.assertFalse(record['success'])
            self.assertIn('error',record)

    def test_nonfinite_and_mismatched_modes_cannot_pass(self):
        coverage={f'{layer}/{scope}':{'equivalent':True} for layer in ('native','app') for scope in ('home','startup','fixture')}
        self.assertTrue(gates(self.measurements(),coverage,True)['acceptance_met'])
        for key,value in [('peak_rss_bytes',float('inf')),('elapsed_ms',float('nan')),('requested_mode','same_process'),('cache_mode','incremental')]:
            runs=self.measurements();runs[0][key]=value
            self.assertEqual(gates(runs,coverage,True)['gates']['native/home/full']['status'],'inconclusive')

    def test_child_reuse_metrics_survive_report_ingestion(self):
        # Exercise the child protocol, including full-scan zero counts and absent
        # metrics from external adapters, rather than assuming every child supports reuse.
        executable={'id':'test','path':sys.executable,'role':'candidate','layer':'native','protocol':'adapter'}
        for metrics in ({'reused_entries':1234,'rescanned_directories':7},
                        {'reused_entries':0,'rescanned_directories':0}, {}):
            payload=json.dumps({'success':True, **metrics})
            executable['args']=['-c',f'print({payload!r})'.replace('{','{{').replace('}','}}')]
            result=run(executable,{'mode':'same_process'},5)
            self.assertTrue(result['success'])
            for key in ('reused_entries','rescanned_directories'):
                self.assertEqual(result[key],metrics.get(key))
                self.assertEqual(result['raw'].get(key),metrics.get(key))

    def test_nonobject_report_is_a_failed_run(self):
        executable={'id':'test','path':sys.executable,'role':'candidate','layer':'native','protocol':'adapter','args':['-c','print("[]")']}
        self.assertFalse(run(executable,{},5)['success'])

    def test_timeout_cannot_pass_after_successful_shutdown(self):
        runs = self.measurements()
        payload = json.dumps(runs[0])
        script = ('import signal, sys, time; '
                  'signal.signal(signal.SIGTERM, lambda *_: sys.exit(0)); '
                  f'print({payload!r}, flush=True); '
                  'print("shutdown pending", file=sys.stderr, flush=True); time.sleep(60)')
        executable = dict(path=sys.executable, layer='native', protocol='adapter',
                          args=['-c', script.replace('{', '{{').replace('}', '}}')])
        result = run(executable, {'mode':'full'}, 1)
        self.assertEqual(result['exit_code'], 0)
        self.assertFalse(result['success'])
        self.assertEqual(result['error'], 'timeout')
        self.assertEqual(result['raw'], runs[0])
        self.assertEqual(result['elapsed_ms'], runs[0]['elapsed_ms'])
        self.assertIn('shutdown pending', result['stderr_tail'])
        runs[0].update(result)
        coverage = {f'{layer}/{scope}':{'equivalent':True}
                    for layer in ('native','app') for scope in ('home','startup','fixture')}
        verdict = gates(runs, coverage, True)
        self.assertEqual(verdict['gates']['native/home/full']['status'], 'inconclusive')
        self.assertFalse(verdict['acceptance_met'])

    def test_stale_linked_scanner_contract_is_a_failed_run(self):
        executable={'id':'test','path':sys.executable,'role':'candidate','layer':'native','protocol':'adapter','expect_exact_path_capacity':True}
        for storage, expected in [({'paths':{'length_bytes':10,'capacity_bytes':20}},False),({'paths':{'length_bytes':10,'capacity_bytes':10}},True),(None,False),({'paths':[]},False)]:
            payload=json.dumps({'storage':storage})
            executable['args']=['-c',f'print({payload!r})'.replace('{','{{').replace('}','}}')]
            result=run(executable,{'mode':'full'},5)
            self.assertEqual(result['success'],expected)
            self.assertEqual(result['source_contract_verified'],expected)

    def test_isolated_modes_and_retry_cannot_claim_full_acceptance(self):
        executable={'id':'candidate','path':sys.executable,'role':'candidate','layer':'native'}
        config={'executables':[executable],'scopes':{'fixture':'/tmp'},'modes':['full','same_process','persisted','changed'],'measurement_modes':['same_process','persisted','changed'],'coverage_scopes':[],'rounds':5}
        requests=[]
        def child(executable, request, timeout):
            requests.append(request.copy())
            state=Path(request['state_dir'])
            # The native guard rejects a nonempty unowned directory.
            marker=state/'owned'
            self.assertTrue(not any(state.iterdir()) or marker.exists())
            marker.touch()
            return {'success':True,'requested_mode':request['mode']}
        checkpoints=[]
        with patch('performance_suite.run',side_effect=child):
            report=execute(config,lambda report: checkpoints.append((len(report['runs']), report['complete'], report['acceptance_met'])))
        states={r['mode']:r['state_dir'] for r in requests}
        self.assertEqual(states['seed'],states['persisted'])
        self.assertEqual(len({states[m] for m in config['measurement_modes']}),3)
        self.assertEqual(len(report['runs']),18)
        self.assertEqual(checkpoints[-1],(18,False,False))
        self.assertTrue(report['complete'])
        self.assertFalse(report['acceptance_met'])
        self.assertNotIn('full',[r['mode'] for r in requests])

    def test_fixture_rescans_and_intervention_are_gated(self):
        runs=self.measurements()
        coverage={f'{layer}/{scope}':{'equivalent':True} for layer in ('native','app') for scope in ('home','startup','fixture')}
        result=gates(runs,coverage,True,modes=('full','same_process'))
        self.assertEqual(result['gates']['native/fixture/same_process']['status'],'inconclusive')
        runs[0]['intervened']=True
        self.assertEqual(gates(runs,coverage,True)['gates']['native/home/full']['status'],'inconclusive')

    def compact_policy(self):
        return {'required_scopes':['fixture','github'], 'memory_scopes':['github'], 'incremental_scopes':['fixture','github']}

    def compact_measurements(self):
        rows=[]
        for layer in ('native','app'):
            for scope in ('fixture','github'):
                for mode in ('full','same_process','persisted','changed'):
                    if mode == 'changed' and scope != 'fixture': continue
                    for role in ('baseline','candidate'):
                        for n in range(6):
                            rows.append(dict(layer=layer,scope=scope,mode=mode,phase='warm' if n else 'initial',role=role,round=n,success=True,coverage_complete=True,requested_mode=mode,cache_mode='full' if mode=='full' else 'incremental',reused_entries=10,preparation_success=True,rendered=True,elapsed_ms=100,workers=3,scheduling={'kind':'unchanged'},storage={'retained_capacity_bytes':100 if role=='baseline' else 60},peak_rss_bytes=100 if role=='baseline' else 75,sampled_peak_aggregate_rss_bytes=100 if role=='baseline' else 75))
        return rows

    def compact_coverage(self):
        return {f'{layer}/{scope}':{'equivalent':True,'compact_cohort_verified':True} for layer in ('native','app') for scope in ('fixture','github')}

    def compact_gates(self, rows=None, coverage=None, **kwargs):
        return gates(self.compact_measurements() if rows is None else rows, self.compact_coverage() if coverage is None else coverage, kwargs.pop('hashes_stable',True), modes=('full','same_process','persisted','changed'), policy=self.compact_policy(), **kwargs)

    def test_compact_policy_cohorts_and_thresholds(self):
        self.assertTrue(self.compact_gates()['acceptance_met'])
        self.assertFalse(self.compact_gates([])['acceptance_met'])
        self.assertFalse(self.compact_gates(coverage={})['acceptance_met'])
        self.assertFalse(self.compact_gates(hashes_stable=False)['acceptance_met'])
        self.assertFalse(self.compact_gates(rounds=4)['acceptance_met'])
        for missing in ('initial','github','persisted','app'):
            rows=[r for r in self.compact_measurements() if missing not in (r['phase'],r['scope'],r['mode'],r['layer'])]
            self.assertFalse(self.compact_gates(rows)['acceptance_met'])
        for field,value in [('elapsed_ms',106),('peak_rss_bytes',81),('storage',{'retained_capacity_bytes':66})]:
            rows=self.compact_measurements()
            for row in rows:
                if row['role']=='candidate' and row['layer']=='native' and row['scope']=='github' and row['mode']=='full': row[field]=value
            self.assertEqual(self.compact_gates(rows)['gates']['native/github/full']['status'],'fail')

    def test_compact_invalid_measurements_cannot_pass(self):
        for field,value in [('cache_mode','full'),('reused_entries',0),('reused_entries',float('inf')),('elapsed_ms',float('nan')),('intervened',True),('workers',4),('scheduling',None),('preparation_success',False),('success',False)]:
            rows=self.compact_measurements()
            row=next(r for r in rows if r['mode']=='persisted' and r['phase']=='warm')
            row[field]=value
            self.assertFalse(self.compact_gates(rows)['acceptance_met'],(field,value))
        coverage=self.compact_coverage();coverage['native/github'].pop('compact_cohort_verified')
        self.assertFalse(self.compact_gates(coverage=coverage)['acceptance_met'])

    def test_compact_contract_and_ingested_storage(self):
        from performance_suite import compact_storage_contract
        storage={'schema_version':2,'layout':'compact_native_v1','node_size_bytes':80,'retained_length_bytes':8,'retained_capacity_bytes':16}
        for name in ('records','native_names','children','path_overrides','display_name_overrides','root','issues','workers'):
            storage[name]={'length_bytes':1,'capacity_bytes':2}
        self.assertTrue(compact_storage_contract(storage))
        chunked = dict(storage, record_storage='chunked_4096_v1',
                       record_chunks={'length_bytes':24,'capacity_bytes':48},
                       clones={'length_bytes':40,'capacity_bytes':80},
                       clone_attributions={'length_bytes':0,'capacity_bytes':0},
                       accounting={'length_bytes':3,'capacity_bytes':7},
                       retained_length_bytes=75, retained_capacity_bytes=151)
        self.assertTrue(compact_storage_contract(chunked))
        for changed in [dict(chunked, record_chunks=None),
                        dict(chunked, record_storage='unknown'),
                        dict(storage, record_storage=None),
                        dict(storage, record_storage=False),
                        dict(storage, record_storage=[]),
                        dict(chunked, clones=None),
                        dict(chunked, retained_capacity_bytes=150),
                        dict(storage, record_chunks={'length_bytes':0,'capacity_bytes':0})]:
            self.assertFalse(compact_storage_contract(changed))
        for field,value in [('layout','legacy'),('node_size_bytes',144),('schema_version',1),('retained_capacity_bytes',17),('workers',{'length_bytes':3,'capacity_bytes':2}),('records',None),('retained_length_bytes',float('inf'))]:
            self.assertFalse(compact_storage_contract({**storage,field:value}))
        executable={'id':'test','path':sys.executable,'role':'candidate','layer':'native','protocol':'adapter','expect_compact_tree':True}
        for payload,expected in [({'storage':storage,'scratch':{'estimated_bytes':9}},True),({'storage':{'paths':{'length_bytes':1,'capacity_bytes':1}}},False)]:
            encoded=json.dumps(payload)
            executable['args']=['-c',f'print({encoded!r})'.replace('{','{{').replace('}','}}')]
            result=run(executable,{'mode':'full'},5)
            self.assertEqual(result['success'],expected)
            self.assertEqual(result['storage'],payload['storage'])
            self.assertEqual(result['scratch'],payload.get('scratch'))

    def test_compact_policy_validation(self):
        from performance_suite import validate
        config={'executables':[{'id':f'{role}-{layer}','path':sys.executable,'role':role,'layer':layer} for role in ('baseline','candidate') for layer in ('native','app')], 'scopes':{'fixture':'/tmp','github':'/tmp'},'modes':['full','same_process','persisted','changed'],'acceptance_policy':self.compact_policy()}
        validate(config)
        for policy in ({}, {'required_scopes':[]}, {**self.compact_policy(),'memory_scopes':[]}, {**self.compact_policy(),'incremental_scopes':['omitted']}):
            with self.assertRaises(ValueError): validate({**config,'acceptance_policy':policy})
        for field,value in [('scopes',{'fixture':'/tmp'}),('modes',['full']),('executables',config['executables'][:2]),('rounds',4)]:
            with self.assertRaises(ValueError): validate({**config,field:value})

    def test_compact_repeated_cross_layer_exports_and_fallback(self):
        executables=[{'id':f'{role}-{layer}','path':sys.executable,'role':role,'layer':layer} for role in ('baseline','candidate') for layer in ('native','app')]
        config={'executables':executables,'scopes':{'fixture':'/tmp','github':'/tmp'},'modes':['full','same_process','persisted','changed'],'acceptance_policy':self.compact_policy()}
        def exercise(fallback=False):
            requests=[]
            def child(executable,request,timeout):
                requests.append((executable['layer'],request.copy()))
                if request.get('coverage_output'): Path(request['coverage_output']).write_text('coverage')
                mode=request['mode']
                return dict(success=True,coverage_complete=True,requested_mode=mode,cache_mode='full' if mode in ('full','seed') or fallback else 'incremental',reused_entries=0 if fallback else 2,rendered=True,workers=3,scheduling={'same':True},elapsed_ms=100,peak_rss_bytes=100 if executable['role']=='baseline' else 75,sampled_peak_aggregate_rss_bytes=100 if executable['role']=='baseline' else 75,storage={'retained_capacity_bytes':100 if executable['role']=='baseline' else 60})
            with patch('performance_suite.run',side_effect=child), patch('performance_suite.compare',return_value={'equivalent':True,'complete':True}), patch('builtins.print'):
                report=execute(config)
            return report,requests
        report,requests=exercise()
        self.assertTrue(report['acceptance_met'])
        self.assertEqual(report['coverage']['native/github']['compact_export_count'],24)
        exports=[(layer,r) for layer,r in requests if r.get('coverage_output') and 'compact-' in r['coverage_output']]
        self.assertEqual(len(exports),48)
        self.assertEqual({layer for layer,_ in exports},{'native','app'})
        self.assertEqual({r['mode'] for _,r in exports},{'full','same_process','persisted'})
        self.assertFalse(exercise(True)[0]['acceptance_met'])

    def test_signaled_child_cannot_succeed(self):
        executable={'id':'signal','path':sys.executable,'role':'candidate','layer':'native','protocol':'adapter','args':['-c','import os,signal; os.kill(os.getpid(),signal.SIGTERM)']}
        self.assertFalse(run(executable,{},5)['success'])

    def test_nr2_all_required_incremental_scopes_cannot_be_omitted(self):
        policy={**self.compact_policy(),'incremental_scopes':['fixture']}
        rows=self.compact_measurements()
        for row in rows:
            if row['scope']=='github' and row['mode'] in ('same_process','persisted'):
                row.update(cache_mode='full',reused_entries=0)
        self.assertFalse(gates(rows,self.compact_coverage(),True,modes=('full','same_process','persisted','changed'),policy=policy)['acceptance_met'])
        self.assertFalse(self.compact_gates(rows)['acceptance_met'])
        for invalid in ({}, {'required_scopes':[]}, {**self.compact_policy(),'incremental_scopes':[]}):
            self.assertFalse(gates(rows,self.compact_coverage(),True,modes=('full','same_process','persisted','changed'),policy=invalid)['acceptance_met'])
        from performance_suite import validate
        config={'executables':[{'id':f'{role}-{layer}','path':sys.executable,'role':role,'layer':layer} for role in ('baseline','candidate') for layer in ('native','app')], 'scopes':{'fixture':'/tmp','github':'/tmp'},'modes':['full','same_process','persisted','changed'],'acceptance_policy':policy}
        with self.assertRaises(ValueError): validate(config)

    def test_nr3_scheduling_frequency_and_rounds_must_match(self):
        rows=self.compact_measurements()
        for row in rows:
            if row['scope']=='github' and row['layer']=='native' and row['mode']=='full' and row['phase']=='warm':
                row['workers']=3 if (row['round'] % 2 == (0 if row['role']=='candidate' else 1)) else 4
        result=self.compact_gates(rows)
        self.assertEqual(result['gates']['native/github/full']['status'],'inconclusive')
        rows=self.compact_measurements()
        for row in rows:
            if row['scope']=='github' and row['layer']=='native' and row['mode']=='full' and row['phase']=='warm':
                row['scheduling']={'policy': 'a' if row['round'] == (1 if row['role']=='baseline' else 2) else 'b'}
        self.assertFalse(self.compact_gates(rows)['acceptance_met'])
        for field,value in [('workers',3.0),('workers',True),('workers',0),('workers',None),('scheduling',None),('scheduling',[]),('scheduling',{}),('scheduling',{'bad':float('nan')}),('scheduling',{'regression':True})]:
            rows=self.compact_measurements()
            row=next(r for r in rows if r['role']=='candidate' and r['phase']=='initial')
            row[field]=value
            self.assertFalse(self.compact_gates(rows)['acceptance_met'],(field,value))
        rows=self.compact_measurements()
        for row in rows:
            row['workers']=3 if row['mode']=='full' else 4
        self.assertTrue(self.compact_gates(rows)['acceptance_met'])
