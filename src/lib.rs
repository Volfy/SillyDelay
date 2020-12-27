#[macro_use]
extern crate vst;
extern crate queues;

use queues::*;
use vst::host::Host;
use vst::buffer::AudioBuffer;
use vst::plugin::{Category, HostCallback, Info, Plugin};
use vst::api::TimeInfo;

// used to make feedback reduce in volume each iteration
const FEEDBACK_FACTOR: f32 = 0.1;

// define the struct for the plugin
struct SillyDelay {
    delay_time: f32,
    dry_wet: f32,
    sample_rate: f32,
    // CircularBuffer is explained later. It will hold a left channel and a right channel, hence the tuple.
    delay_buffer: CircularBuffer::<(f32, f32)>,
    feedback_amt: f32,
}

impl Default for SillyDelay {

    // This is somehow necessary, but doesn't really do much since we initialize later anyway
    fn default() -> SillyDelay {
        SillyDelay {
            delay_buffer: reload_delay_buffer(44100., 0.001),
            delay_time: 0.001,
            dry_wet: 1.0,
            sample_rate: 44100.,
            feedback_amt: 0.1,
        }
    }
}

// implement Plugin for SillyDelay
impl Plugin for SillyDelay {

    // initialize
    fn new(host: HostCallback) -> Self {

        // Get the sample rate immediately before anything else
        // In order to set the sample rate in the case that it's not changed
        // use get_time_info with no flags. Sample rate is always valid in TimeInfo
        // Possible improvement: set Tempo flag and use Tempo with sample rate for Synced delay times.
        let sample_rate = if let Some(time_info) = host.get_time_info(0) {
            match time_info {
                TimeInfo { sample_rate, ..} => sample_rate as f32
            }
            // will never get to else clause
        } else { 0.0 };

        SillyDelay {
            delay_time: 0.001,
            dry_wet: 1.0,
            sample_rate: sample_rate,
            delay_buffer: reload_delay_buffer(sample_rate, 0.001),
            feedback_amt: 0.1,
        }
    }

    // necessary for Plugin trait
    fn get_info(&self) -> Info {
        Info { 
            parameters: 3,
            inputs: 2,
            outputs: 2,
            category: Category::Effect,
            f64_precision: false,

            name: "SillyDelay".to_string(),
            vendor: "Volfym".to_string(),
            // randomly generated online. necessary to work
            unique_id: 486893,

            ..Info::default()
        }
    }

    // sets parameters when host changes them.
    fn set_parameter(&mut self, index: i32, value: f32) {
        match index {
            // delay time. delay_buffer is also reloaded. Because of this it's not possible to have a smooth change
            // between one delay time and another. To prevent any issues when loading delay_buffer
            // delay time cannot be zero.
            0 => {
                self.delay_time = value.max(0.001);
                self.delay_buffer = reload_delay_buffer(self.sample_rate, self.delay_time);
            },
            // I don't want any problems below FEEDBACK_FACTOR value, so minimum cap of feedback is 10%
            // although in reality that is equivalent to 0 feedback.
            1 => self.feedback_amt = value.max(0.1),
            2 => self.dry_wet = value,
            _ => (),
        }
    }

    // provides params when host asks for them.
    fn get_parameter(&self, index: i32) -> f32 {
       match index {
           0 => self.delay_time,
           1 => self.feedback_amt,
           2 => self.dry_wet,
           _ => 0.0,
       }
    }

    // provides param names when host asks for them.
    fn get_parameter_name(&self, index: i32) -> String {
        match index {
            0 => "Delay Time".to_string(),
            1 => "Feedback".to_string(),
            2 => "Dry/Wet".to_string(),
            _ => "".to_string(),
        }
    }

    // param value text.
    fn get_parameter_text(&self, index: i32) -> String {
        match index {
            // all params go from 0 to 1. Delay time is multiplied by two later
            // because I wanted a longer delay time. 
            0 => format!("{}", self.delay_time * 2000.0),
            1 => format!("{}", self.feedback_amt * 100.0),
            2 => format!("{}", self.dry_wet * 100.0),
            _ => "".to_string(),
        }
    }

    // param labels. 
    fn get_parameter_label(&self, index: i32) -> String {
        match index {
            0 => "ms".to_string(),
            1 => "%".to_string(),
            2 => "%".to_string(),
            _ => "".to_string(),
        }
    }

    // in the case that the host changes sample rate
    // this function is called. We update the sample_rate held in SillyDelay
    // and also reload the delay_buffer to reflect the new sample_rate
    fn set_sample_rate(&mut self, sample_rate: f32) {
        self.sample_rate = sample_rate;
        self.delay_buffer = reload_delay_buffer(sample_rate, self.delay_time);
        
    }

    // main processing goes here
    fn process(&mut self, buffer: &mut AudioBuffer<f32>) {
        // stores feedback values for later
        // needs to be mutable and set to 0 or it won't work
        let (mut fb_l, mut fb_r) = (0f32, 0f32);

        // split the audio buffer, and split the inputs buffer
        // into left and right channels with split_at() 
        // outputs has to be mutable borrow 
        let (inputs, mut outputs) = buffer.split();
        let (in_l, in_r) = inputs.split_at(1);
    
        // split the outputs buffer into left and right channels, both mutable
        let (out_l, out_r) = outputs.split_at_mut(1);

        // ok this is weird. There is definitely a better way of doing this.
        // in order to get to the samples, channels need to be further destructured
        // since there's 4 things to iterate/zip over it would have been an issue to nest 
        // a for loop within another. 
        // each zip adds to a tuple, going outwards, hence the weird (((x,x),x)x) thing.
        // since we have "Inputs" or "Outputs" types, into_iter is necessary 
        // to get InputIterators and OutputIterators 

        // sidenote: l / r is left, right; b is buffer, s is sample.
        for (((in_l_b, in_r_b), out_l_b), out_r_b) in in_l
        .into_iter()
        .zip(in_r.into_iter())
        .zip(out_l.into_iter())
        .zip(out_r.into_iter())
        {
            // repeat the process to get the values from the slices
            for (((in_l_s, in_r_s), out_l_s), out_r_s) in in_l_b
            .iter()
            .zip(in_r_b)
            .zip(out_l_b)
            .zip(out_r_b)
            {
                // delay_buffer is a CircularBuffer 
                // it has a maximum size, and each time something is added, it will pop the next thing in queue
                // First In First Out. Because delay_buffer is immediately filled in with 0s there's no case where
                // adding something will return None
                if let Some((temp_l, temp_r)) = self.delay_buffer
                // dereference the inputs (in_l_s, in_r_s) to get the values, and add the feedback 
                .add((*in_l_s+fb_l, *in_r_s+fb_r))
                // convert the Result into an Option and discard error and then get the tuple value returned to (temp_l, temp_r)
                .ok().unwrap() {
                    // if successful (ie, there is Some(value))
                    // add popped values from delay_buffer into feedback variables
                    // feedback_amt - FEEDBACK_FACTOR always ensures the value is between 
                    // 0 and 0.9 - to prevent, well, too much feedback
                    fb_l = temp_l * (self.feedback_amt - FEEDBACK_FACTOR);
                    fb_r = temp_r * (self.feedback_amt - FEEDBACK_FACTOR);

                    // replace the output samples with a mix of the popped values from the delay_buffer
                    // and the original value, depending on dry/wet percentage
                    // Possible expansion: Allow possibility to have unsynced left and right delays
                    *out_l_s = mix_samples(*out_l_s, temp_l, self.dry_wet);
                    *out_r_s = mix_samples(*out_r_s, temp_r, self.dry_wet);
                }

            }
        }
     }
}

fn mix_samples(original: f32, added: f32, amount: f32) -> f32 {
    // always ensures that there's never more than 100%
    // if dry_wet (amount) is 60%, dry amount is 0.4, wet is 0.6
    let dry = 1.0 - amount;
    // return the mixed value
    (original*dry) + (added*amount)
}

fn reload_delay_buffer(sample_rate: f32, delay_time: f32) -> CircularBuffer<(f32, f32)> {
    // by having this in one place, it reduces the amount of places where CircularBuffer is called
    // and it doesn't need to have access to delay_time or sample_rate directly from SillyDelay
    // in case, for example, they're not initialized yet
    // A problem with this is that any time delay time is changed the whole buffer is cleaned out.

    // A sample rate is always in (kilo)Hertz, ie. per Second. Pretty obvious, but I forgot for a moment earlier.
    // So to ensure a maximum of 2 seconds - the size of our delay_buffer has to be the sample rate times 2.
    // (delay_time can only go up to 1.0 maximum)
    // if the delay time chosen is less than that, for example, 200ms, we need to use a smaller delay_buffer
    // hence rate * time * 2
    let size = (sample_rate * delay_time * 2.) as usize;
    // buffer is immediately populated with tuples of 0,0 so that each .add pops Some(value)
    // tuple because (left, right)
    CircularBuffer::with_default(size, (0f32, 0f32))
}

// necessary to compile to VST
plugin_main!(SillyDelay);