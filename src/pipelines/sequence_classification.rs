// Copyright 2019-present, the HuggingFace Inc. team, The Google AI Language Team and Facebook, Inc.
// Copyright 2019-2020 Guillaume Becquin
// Copyright 2020 Maarten van Gompel
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//     http://www.apache.org/licenses/LICENSE-2.0
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.
//! # Sequence classification pipeline (e.g. Sentiment Analysis)
//! More generic sequence classification pipeline, works with multiple models (Bert, Roberta)
//!
//! ```no_run
//! use rust_bert::pipelines::sequence_classification::SequenceClassificationConfig;
//! use rust_bert::resources::{RemoteResource, Resource};
//! use rust_bert::distilbert::{DistilBertModelResources, DistilBertVocabResources, DistilBertConfigResources};
//! use rust_bert::pipelines::sequence_classification::SequenceClassificationModel;
//! use rust_bert::pipelines::common::ModelType;
//! # fn main() -> anyhow::Result<()> {
//!
//! //Load a configuration
//! let config = SequenceClassificationConfig::new(ModelType::DistilBert,
//!    Resource::Remote(RemoteResource::from_pretrained(DistilBertModelResources::DISTIL_BERT_SST2)),
//!    Resource::Remote(RemoteResource::from_pretrained(DistilBertVocabResources::DISTIL_BERT_SST2)),
//!    Resource::Remote(RemoteResource::from_pretrained(DistilBertConfigResources::DISTIL_BERT_SST2)),
//!    None, //merges resource only relevant with ModelType::Roberta
//!    true, //lowercase
//!    None, //strip_accents
//!    None, //add_prefix_space
//! );
//!
//! //Create the model
//! let sequence_classification_model = SequenceClassificationModel::new(config)?;
//!
//! let input = [
//!     "Probably my all-time favorite movie, a story of selflessness, sacrifice and dedication to a noble cause, but it's not preachy or boring.",
//!     "This film tried to be too many things all at once: stinging political satire, Hollywood blockbuster, sappy romantic comedy, family values promo...",
//!     "If you like original gut wrenching laughter you will like this movie. If you are young or old then you will love this movie, hell even my mom liked it.",
//! ];
//! let output = sequence_classification_model.predict(&input);
//! # Ok(())
//! # }
//! ```
//! (Example courtesy of [IMDb](http://www.imdb.com))
//!
//! Output: \
//! ```no_run
//! # use rust_bert::pipelines::sequence_classification::Label;
//! let output =
//! [
//!    Label { text: String::from("POSITIVE"), score: 0.9986, id: 1, sentence: 0},
//!    Label { text: String::from("NEGATIVE"), score: 0.9985, id: 0, sentence: 1},
//!    Label { text: String::from("POSITIVE"), score: 0.9988, id: 1, sentence: 12},
//! ]
//! # ;
//! ```
use crate::albert::AlbertForSequenceClassification;
use crate::bart::BartForSequenceClassification;
use crate::bert::BertForSequenceClassification;
use crate::common::error::RustBertError;
use crate::common::resources::{RemoteResource, Resource};
use crate::distilbert::{
    DistilBertConfigResources, DistilBertModelClassifier, DistilBertModelResources,
    DistilBertVocabResources,
};
use crate::pipelines::common::{ConfigOption, ModelType, TokenizerOption};
use crate::roberta::RobertaForSequenceClassification;
use rust_tokenizers::preprocessing::tokenizer::base_tokenizer::{
    TokenizedInput, TruncationStrategy,
};
use serde::{Deserialize, Serialize};
use std::borrow::Borrow;
use std::collections::HashMap;
use tch::nn::VarStore;
use tch::{nn, no_grad, Device, Kind, Tensor};

#[derive(Debug, Serialize, Deserialize, Clone)]
/// # Label generated by a `SequenceClassificationModel`
pub struct Label {
    /// Label String representation
    pub text: String,
    /// Confidence score
    pub score: f64,
    /// Label ID
    pub id: i64,
    /// Sentence index
    #[serde(default)]
    pub sentence: usize,
}

/// # Configuration for SequenceClassificationModel
/// Contains information regarding the model to load and device to place the model on.
pub struct SequenceClassificationConfig {
    /// Model type
    pub model_type: ModelType,
    /// Model weights resource (default: pretrained BERT model on CoNLL)
    pub model_resource: Resource,
    /// Config resource (default: pretrained BERT model on CoNLL)
    pub config_resource: Resource,
    /// Vocab resource (default: pretrained BERT model on CoNLL)
    pub vocab_resource: Resource,
    /// Merges resource (default: None)
    pub merges_resource: Option<Resource>,
    /// Automatically lower case all input upon tokenization (assumes a lower-cased model)
    pub lower_case: bool,
    /// Flag indicating if the tokenizer should strip accents (normalization). Only used for BERT / ALBERT models
    pub strip_accents: Option<bool>,
    /// Flag indicating if the tokenizer should add a white space before each tokenized input (needed for some Roberta models)
    pub add_prefix_space: Option<bool>,
    /// Device to place the model on (default: CUDA/GPU when available)
    pub device: Device,
}

impl SequenceClassificationConfig {
    /// Instantiate a new sequence classification configuration of the supplied type.
    ///
    /// # Arguments
    ///
    /// * `model_type` - `ModelType` indicating the model type to load (must match with the actual data to be loaded!)
    /// * model - The `Resource` pointing to the model to load (e.g.  model.ot)
    /// * config - The `Resource' pointing to the model configuration to load (e.g. config.json)
    /// * vocab - The `Resource' pointing to the tokenizer's vocabulary to load (e.g.  vocab.txt/vocab.json)
    /// * vocab - An optional `Resource` tuple (`Option<Resource>`) pointing to the tokenizer's merge file to load (e.g.  merges.txt), needed only for Roberta.
    /// * lower_case - A `bool' indicating whether the tokeniser should lower case all input (in case of a lower-cased model)
    pub fn new(
        model_type: ModelType,
        model_resource: Resource,
        config_resource: Resource,
        vocab_resource: Resource,
        merges_resource: Option<Resource>,
        lower_case: bool,
        strip_accents: impl Into<Option<bool>>,
        add_prefix_space: impl Into<Option<bool>>,
    ) -> SequenceClassificationConfig {
        SequenceClassificationConfig {
            model_type,
            model_resource,
            config_resource,
            vocab_resource,
            merges_resource,
            lower_case,
            strip_accents: strip_accents.into(),
            add_prefix_space: add_prefix_space.into(),
            device: Device::cuda_if_available(),
        }
    }
}

impl Default for SequenceClassificationConfig {
    /// Provides a defaultSST-2 sentiment analysis model (English)
    fn default() -> SequenceClassificationConfig {
        SequenceClassificationConfig {
            model_type: ModelType::DistilBert,
            model_resource: Resource::Remote(RemoteResource::from_pretrained(
                DistilBertModelResources::DISTIL_BERT_SST2,
            )),
            config_resource: Resource::Remote(RemoteResource::from_pretrained(
                DistilBertConfigResources::DISTIL_BERT_SST2,
            )),
            vocab_resource: Resource::Remote(RemoteResource::from_pretrained(
                DistilBertVocabResources::DISTIL_BERT_SST2,
            )),
            merges_resource: None,
            lower_case: true,
            strip_accents: None,
            add_prefix_space: None,
            device: Device::cuda_if_available(),
        }
    }
}

/// # Abstraction that holds one particular sequence classification model, for any of the supported models
pub enum SequenceClassificationOption {
    /// Bert for Sequence Classification
    Bert(BertForSequenceClassification),
    /// DistilBert for Sequence Classification
    DistilBert(DistilBertModelClassifier),
    /// Roberta for Sequence Classification
    Roberta(RobertaForSequenceClassification),
    /// XLMRoberta for Sequence Classification
    XLMRoberta(RobertaForSequenceClassification),
    /// Albert for Sequence Classification
    Albert(AlbertForSequenceClassification),
    /// Bart for Sequence Classification
    Bart(BartForSequenceClassification),
}

impl SequenceClassificationOption {
    /// Instantiate a new sequence classification model of the supplied type.
    ///
    /// # Arguments
    ///
    /// * `model_type` - `ModelType` indicating the model type to load (must match with the actual data to be loaded)
    /// * `p` - `tch::nn::Path` path to the model file to load (e.g. model.ot)
    /// * `config` - A configuration (the model type of the configuration must be compatible with the value for
    /// `model_type`)
    pub fn new<'p, P>(model_type: ModelType, p: P, config: &ConfigOption) -> Self
    where
        P: Borrow<nn::Path<'p>>,
    {
        match model_type {
            ModelType::Bert => {
                if let ConfigOption::Bert(config) = config {
                    SequenceClassificationOption::Bert(BertForSequenceClassification::new(
                        p, config,
                    ))
                } else {
                    panic!("You can only supply a BertConfig for Bert!");
                }
            }
            ModelType::DistilBert => {
                if let ConfigOption::DistilBert(config) = config {
                    SequenceClassificationOption::DistilBert(DistilBertModelClassifier::new(
                        p, config,
                    ))
                } else {
                    panic!("You can only supply a DistilBertConfig for DistilBert!");
                }
            }
            ModelType::Roberta => {
                if let ConfigOption::Bert(config) = config {
                    SequenceClassificationOption::Roberta(RobertaForSequenceClassification::new(
                        p, config,
                    ))
                } else {
                    panic!("You can only supply a BertConfig for Roberta!");
                }
            }
            ModelType::XLMRoberta => {
                if let ConfigOption::Bert(config) = config {
                    SequenceClassificationOption::XLMRoberta(RobertaForSequenceClassification::new(
                        p, config,
                    ))
                } else {
                    panic!("You can only supply a BertConfig for Roberta!");
                }
            }
            ModelType::Albert => {
                if let ConfigOption::Albert(config) = config {
                    SequenceClassificationOption::Albert(AlbertForSequenceClassification::new(
                        p, config,
                    ))
                } else {
                    panic!("You can only supply an AlbertConfig for Albert!");
                }
            }
            ModelType::Bart => {
                if let ConfigOption::Bart(config) = config {
                    SequenceClassificationOption::Bart(BartForSequenceClassification::new(
                        p, config,
                    ))
                } else {
                    panic!("You can only supply a BertConfig for Bert!");
                }
            }
            ModelType::Electra => {
                panic!("SequenceClassification not implemented for Electra!");
            }
            ModelType::Marian => {
                panic!("SequenceClassification not implemented for Marian!");
            }
            ModelType::T5 => {
                panic!("SequenceClassification not implemented for T5!");
            }
        }
    }

    /// Returns the `ModelType` for this SequenceClassificationOption
    pub fn model_type(&self) -> ModelType {
        match *self {
            Self::Bert(_) => ModelType::Bert,
            Self::Roberta(_) => ModelType::Roberta,
            Self::XLMRoberta(_) => ModelType::Roberta,
            Self::DistilBert(_) => ModelType::DistilBert,
            Self::Albert(_) => ModelType::Albert,
            Self::Bart(_) => ModelType::Bart,
        }
    }

    /// Interface method to forward_t() of the particular models.
    pub fn forward_t(
        &self,
        input_ids: Option<Tensor>,
        mask: Option<Tensor>,
        token_type_ids: Option<Tensor>,
        position_ids: Option<Tensor>,
        input_embeds: Option<Tensor>,
        train: bool,
    ) -> Tensor {
        match *self {
            Self::Bart(ref model) => {
                model
                    .forward_t(
                        &input_ids.expect("`input_ids` must be provided for BART models"),
                        mask.as_ref(),
                        None,
                        None,
                        None,
                        train,
                    )
                    .decoder_output
            }
            Self::Bert(ref model) => {
                model
                    .forward_t(
                        input_ids,
                        mask,
                        token_type_ids,
                        position_ids,
                        input_embeds,
                        train,
                    )
                    .logits
            }
            Self::DistilBert(ref model) => {
                model
                    .forward_t(input_ids, mask, input_embeds, train)
                    .expect("Error in distilbert forward_t")
                    .logits
            }
            Self::Roberta(ref model) | Self::XLMRoberta(ref model) => {
                model
                    .forward_t(
                        input_ids,
                        mask,
                        token_type_ids,
                        position_ids,
                        input_embeds,
                        train,
                    )
                    .0
            }
            Self::Albert(ref model) => {
                model
                    .forward_t(
                        input_ids,
                        mask,
                        token_type_ids,
                        position_ids,
                        input_embeds,
                        train,
                    )
                    .logits
            }
        }
    }
}

/// # SequenceClassificationModel for Classification (e.g. Sentiment Analysis)
pub struct SequenceClassificationModel {
    tokenizer: TokenizerOption,
    sequence_classifier: SequenceClassificationOption,
    label_mapping: HashMap<i64, String>,
    var_store: VarStore,
}

impl SequenceClassificationModel {
    /// Build a new `SequenceClassificationModel`
    ///
    /// # Arguments
    ///
    /// * `config` - `SequenceClassificationConfig` object containing the resource references (model, vocabulary, configuration) and device placement (CPU/GPU)
    ///
    /// # Example
    ///
    /// ```no_run
    /// # fn main() -> anyhow::Result<()> {
    /// use rust_bert::pipelines::sequence_classification::SequenceClassificationModel;
    ///
    /// let model = SequenceClassificationModel::new(Default::default())?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn new(
        config: SequenceClassificationConfig,
    ) -> Result<SequenceClassificationModel, RustBertError> {
        let config_path = config.config_resource.get_local_path()?;
        let vocab_path = config.vocab_resource.get_local_path()?;
        let weights_path = config.model_resource.get_local_path()?;
        let merges_path = if let Some(merges_resource) = &config.merges_resource {
            Some(merges_resource.get_local_path()?)
        } else {
            None
        };
        let device = config.device;

        let tokenizer = TokenizerOption::from_file(
            config.model_type,
            vocab_path.to_str().unwrap(),
            merges_path.as_deref().map(|path| path.to_str().unwrap()),
            config.lower_case,
            config.strip_accents,
            config.add_prefix_space,
        )?;
        let mut var_store = VarStore::new(device);
        let model_config = ConfigOption::from_file(config.model_type, config_path);
        let sequence_classifier =
            SequenceClassificationOption::new(config.model_type, &var_store.root(), &model_config);
        let label_mapping = model_config.get_label_mapping();
        var_store.load(weights_path)?;
        Ok(SequenceClassificationModel {
            tokenizer,
            sequence_classifier,
            label_mapping,
            var_store,
        })
    }

    fn prepare_for_model(&self, input: Vec<&str>) -> Tensor {
        let tokenized_input: Vec<TokenizedInput> =
            self.tokenizer
                .encode_list(input.to_vec(), 128, &TruncationStrategy::LongestFirst, 0);
        let max_len = tokenized_input
            .iter()
            .map(|input| input.token_ids.len())
            .max()
            .unwrap();
        let tokenized_input_tensors: Vec<tch::Tensor> = tokenized_input
            .iter()
            .map(|input| input.token_ids.clone())
            .map(|mut input| {
                input.extend(vec![
                    self.tokenizer.get_pad_id().expect(
                        "The Tokenizer used for sequence classification should contain a PAD id"
                    );
                    max_len - input.len()
                ]);
                input
            })
            .map(|input| Tensor::of_slice(&(input)))
            .collect::<Vec<_>>();
        Tensor::stack(tokenized_input_tensors.as_slice(), 0).to(self.var_store.device())
    }

    /// Classify texts
    ///
    /// # Arguments
    ///
    /// * `input` - `&[&str]` Array of texts to classify.
    ///
    /// # Returns
    ///
    /// * `Vec<Label>` containing labels for input texts
    ///
    /// # Example
    ///
    /// ```no_run
    /// # fn main() -> anyhow::Result<()> {
    /// # use rust_bert::pipelines::sequence_classification::SequenceClassificationModel;
    ///
    /// let sequence_classification_model =  SequenceClassificationModel::new(Default::default())?;
    /// let input = [
    ///     "Probably my all-time favorite movie, a story of selflessness, sacrifice and dedication to a noble cause, but it's not preachy or boring.",
    ///     "This film tried to be too many things all at once: stinging political satire, Hollywood blockbuster, sappy romantic comedy, family values promo...",
    ///     "If you like original gut wrenching laughter you will like this movie. If you are young or old then you will love this movie, hell even my mom liked it.",
    /// ];
    /// let output = sequence_classification_model.predict(&input);
    /// # Ok(())
    /// # }
    /// ```
    pub fn predict(&self, input: &[&str]) -> Vec<Label> {
        let input_tensor = self.prepare_for_model(input.to_vec());
        let output = no_grad(|| {
            let output = self.sequence_classifier.forward_t(
                Some(input_tensor.copy()),
                None,
                None,
                None,
                None,
                false,
            );
            output.softmax(-1, Kind::Float).detach().to(Device::Cpu)
        });
        let label_indices = output.as_ref().argmax(-1, true).squeeze1(1);
        let scores = output
            .gather(1, &label_indices.unsqueeze(-1), false)
            .squeeze1(1);
        let label_indices = label_indices.iter::<i64>().unwrap().collect::<Vec<i64>>();
        let scores = scores.iter::<f64>().unwrap().collect::<Vec<f64>>();

        let mut labels: Vec<Label> = vec![];
        for sentence_idx in 0..label_indices.len() {
            let label_string = self
                .label_mapping
                .get(&label_indices[sentence_idx])
                .unwrap()
                .clone();
            let label = Label {
                text: label_string,
                score: scores[sentence_idx],
                id: label_indices[sentence_idx],
                sentence: sentence_idx,
            };
            labels.push(label)
        }
        labels
    }

    /// Multi-label classification of texts
    ///
    /// # Arguments
    ///
    /// * `input` - `&[&str]` Array of texts to classify.
    /// * `threshold` - `f64` threshold above which a label will be considered true by the classifier
    ///
    /// # Returns
    ///
    /// * `Vec<Vec<Label>>` containing a vector of true labels for each input text
    ///
    /// # Example
    ///
    /// ```no_run
    /// # fn main() -> anyhow::Result<()> {
    /// # use rust_bert::pipelines::sequence_classification::SequenceClassificationModel;
    ///
    /// let sequence_classification_model =  SequenceClassificationModel::new(Default::default())?;
    /// let input = [
    ///     "Probably my all-time favorite movie, a story of selflessness, sacrifice and dedication to a noble cause, but it's not preachy or boring.",
    ///     "This film tried to be too many things all at once: stinging political satire, Hollywood blockbuster, sappy romantic comedy, family values promo...",
    ///     "If you like original gut wrenching laughter you will like this movie. If you are young or old then you will love this movie, hell even my mom liked it.",
    /// ];
    /// let output = sequence_classification_model.predict_multilabel(&input, 0.5);
    /// # Ok(())
    /// # }
    /// ```
    pub fn predict_multilabel(
        &self,
        input: &[&str],
        threshold: f64,
    ) -> Result<Vec<Vec<Label>>, RustBertError> {
        let input_tensor = self.prepare_for_model(input.to_vec());
        let output = no_grad(|| {
            let output = self.sequence_classifier.forward_t(
                Some(input_tensor.copy()),
                None,
                None,
                None,
                None,
                false,
            );
            output.sigmoid().detach().to(Device::Cpu)
        });
        let label_indices = output.as_ref().ge(threshold).nonzero();

        let mut labels: Vec<Vec<Label>> = vec![];
        let mut sequence_labels: Vec<Label> = vec![];

        for sentence_idx in 0..label_indices.size()[0] {
            let label_index_tensor = label_indices.get(sentence_idx);
            let sentence_label = label_index_tensor
                .iter::<i64>()
                .unwrap()
                .collect::<Vec<i64>>();
            let (sentence, id) = (sentence_label[0], sentence_label[1]);
            if sentence as usize > labels.len() {
                labels.push(sequence_labels);
                sequence_labels = vec![];
            }
            let score = output.double_value(sentence_label.as_slice());
            let label_string = self.label_mapping.get(&id).unwrap().to_owned();
            let label = Label {
                text: label_string,
                score,
                id,
                sentence: sentence as usize,
            };
            sequence_labels.push(label);
        }
        if !sequence_labels.is_empty() {
            labels.push(sequence_labels);
        }
        Ok(labels)
    }
}
